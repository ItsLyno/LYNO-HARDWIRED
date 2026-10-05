import { AppWindow, Check, Download, FileArchive, FolderOpen, Layers, Loader2, RotateCw, Search } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { api, type Folder, type Settings as SettingsT } from "../api";
import { Button } from "../components/Button";
import { InstanceSetup } from "../components/InstanceSetup";
import { Integrity } from "../components/Integrity";
import { NexusAccount } from "../components/NexusAccount";
import { PageTitle } from "../components/PageTitle";
import { downloadLabel, plural } from "../format";
import { activeInstance, isRepairOnly, useApp } from "../store";

export function Settings() {
  const { settings, saveSettings, run, setPage } = useApp();
  const [draft, setDraft] = useState<SettingsT | null>(settings);
  const [saved, setSaved] = useState(false);
  const [detectNote, setDetectNote] = useState<string | null>(null);
  const [report, setReport] = useState<{ busy: boolean; path: string | null }>({ busy: false, path: null });

  useEffect(() => {
    if (settings && !draft) setDraft(settings);
  }, [settings, draft]);
  if (!draft) return null;

  // Instances and the check interval change through their own calls, not the Save button.
  const editable = (s: SettingsT | null) => s && { ...s, instanceDir: null, instances: null, updateCheckHours: null };
  const dirty = JSON.stringify(editable(draft)) !== JSON.stringify(editable(settings));
  const isBuild = !!activeInstance(settings)?.build;
  // Applied at once, not through the Save button; the draft follows so a later save doesn't undo it.
  const disableAuthorMode = () =>
    run(async () => {
      if (settings && (await saveSettings({ ...settings, authorMode: false }))) setDraft({ ...draft, authorMode: false });
    });
  const authorEnabled = () => {
    setDraft({ ...draft, authorMode: true });
    setPage("release");
  };
  const update = (patch: Partial<SettingsT>) => {
    setDraft({ ...draft, ...patch });
    setSaved(false);
  };

  const detect = () =>
    run(async () => {
      const found = await api.detectGames();
      if (found.length > 0) {
        update({ gameDir: found[0].path });
        setDetectNote(`Найдено: ${found.map((g) => g.store).join(", ")}`);
      } else {
        setDetectNote("Игра не найдена в Steam, GOG и Epic. Укажите папку вручную.");
      }
    });

  const exportReport = () =>
    run(async () => {
      setReport({ busy: true, path: null });
      try {
        setReport({ busy: false, path: await api.exportReport() });
      } catch (e) {
        setReport({ busy: false, path: null });
        throw e;
      }
    });

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <PageTitle>Настройки</PageTitle>
      <Profiles />
      <h2 className="pt-4 text-base font-semibold">Игра</h2>
      <div className="panel divide-y divide-line/60">
        <Field label="Папка Cyberpunk 2077" hint="Папка, в которой лежит bin\x64\Cyberpunk2077.exe.">
          <div className="flex gap-2">
            <Input value={draft.gameDir ?? ""} onChange={(v) => update({ gameDir: v || null })} />
            <Button onClick={detect}>
              <Search size={15} />
              Найти
            </Button>
          </div>
          {detectNote && <div className="mt-2 text-[13px] text-muted">{detectNote}</div>}
        </Field>
        {settings?.authorMode && (
          <>
            <Field
              label="Режим автора"
              hint="Вы выпускаете сборку из этой папки: правки модов считаются следующим выпуском, обновление и починка выключены. Выключите, чтобы снова получать опубликованные версии как игрок."
            >
              <Button onClick={disableAuthorMode}>Выключить режим автора</Button>
            </Field>
            <Field label="Репозиторий сборки" hint="Куда публикуются выпуски: владелец/репозиторий на GitHub.">
              <Input value={draft.authorRepo} onChange={(v) => update({ authorRepo: v })} />
            </Field>
            <Field
              label="Папка для упаковки"
              hint="Сюда складываются части новой версии перед загрузкой. Пусто — папка release-out рядом с папкой сборки. Не указывайте папку внутри сборки; быстрее всего на другом диске."
            >
              <Input value={draft.authorOutDir ?? ""} onChange={(v) => update({ authorOutDir: v || null })} />
            </Field>
          </>
        )}
      </div>
      <div className="flex items-center gap-3">
        <Button
          variant="primary"
          disabled={!dirty}
          onClick={async () => {
            if (settings && (await saveSettings({ ...draft, updateCheckHours: settings.updateCheckHours }))) setSaved(true);
          }}
        >
          Сохранить
        </Button>
        {saved && !dirty && (
          <span className="flex items-center gap-1.5 text-[13px] text-muted">
            <Check size={15} className="text-ok" />
            Сохранено
          </span>
        )}
      </div>
      <h2 className="pt-4 text-base font-semibold">Обновления</h2>
      <Updates />
      <h2 className="pt-4 text-base font-semibold">Nexus Mods</h2>
      <NexusAccount />
      <h2 className="pt-4 text-base font-semibold">Папки</h2>
      <div className="panel">
        <Field
          label="Открыть в проводнике"
          hint="Сохранения лежат в обычной папке игры и общие с игрой без модов. Логи лаунчера пригодятся, если что-то пошло не так при установке."
        >
          <div className="flex flex-wrap gap-2">
            {folders.map(([folder, label]) => (
              <Button key={folder} onClick={() => run(() => api.openFolder(folder))}>
                <FolderOpen size={15} />
                {label}
              </Button>
            ))}
          </div>
        </Field>
      </div>
      <h2 className="pt-4 text-base font-semibold">Диагностика</h2>
      <div className="panel divide-y divide-line/60">
        {!isBuild ? null : settings?.authorMode ? (
          <Field
            label="Проверка целостности"
            hint="В режиме автора починка выключена: изменённые файлы — это ваши правки. Посмотреть их можно на вкладке «Выпуск»."
          >
            <Button onClick={() => setPage("release")}>Открыть «Выпуск»</Button>
          </Field>
        ) : (
          <Field
            label="Проверка целостности"
            hint="Лаунчер перечитает все файлы модов сборки и сравнит их с установленными. Это займёт столько же времени, сколько чтение всей сборки с диска. Повреждённый мод скачается заново целиком, а настройки, которые вы меняли в игре, сохранятся."
          >
            <Integrity />
          </Field>
        )}
        <Field
          label="Отчёт для автора сборки"
          hint="Если игра вылетает или сборка не обновляется, соберите отчёт и отправьте файл автору. В архив попадут логи лаунчера, MO2, RED4ext, CET и redscript, список модов и состояние установки. Сохранений и настроек графики в нём нет."
        >
          <div className="flex items-center gap-3">
            <Button onClick={exportReport} disabled={report.busy}>
              {report.busy ? <Loader2 size={15} className="animate-spin" /> : <FileArchive size={15} />}
              Собрать отчёт
            </Button>
            {report.path && (
              <span className="min-w-0 truncate font-mono text-[13px] text-muted" title={report.path}>
                {report.path}
              </span>
            )}
          </div>
        </Field>
      </div>
      <p className="pt-4 font-mono text-[11px] text-faint">
        LYNO//HARDWIRED · Моды принадлежат их авторам.
      </p>
      {!settings?.authorMode && <AuthorUnlock repo={settings?.authorRepo ?? ""} onEnabled={authorEnabled} />}
    </div>
  );
}

/** One card per MO2 instance: the active one is marked, the others switch with a click. */
function Profiles() {
  const { settings, switchInstance, progress } = useApp();
  const instances = settings?.instances ?? [];
  return (
    <section className="space-y-3">
      <div>
        <h2 className="text-base font-semibold">Профиль Mod Organizer 2</h2>
        <p className="mt-0.5 text-[13px] text-muted">
          Каждый профиль — отдельная папка MO2 со своими модами. Кнопка «Играть» и список модов работают с активным профилем.
        </p>
      </div>
      <div className="grid grid-cols-2 gap-3">
        {instances.map((i) => {
          const active = i.dir === settings?.instanceDir;
          return (
            <button
              key={i.dir}
              disabled={active || !!progress}
              onClick={() => switchInstance(() => api.instanceSelect(i.dir))}
              className={`panel flex flex-col items-start gap-1 px-5 py-4 text-left transition-colors ${
                active ? "ring-1 ring-neon" : "hover:bg-raised/60 disabled:opacity-60"
              }`}
            >
              <span className="flex w-full items-center gap-2">
                <span className="truncate text-sm font-medium">{i.name}</span>
                <span className="rounded-full bg-raised px-2 py-0.5 text-[11px] text-muted">{i.build ? "сборка" : "свой MO2"}</span>
                <span className="flex-1" />
                {active ? (
                  <span className="flex items-center gap-1 text-[12px] text-neon">
                    <Check size={13} />
                    Активный
                  </span>
                ) : (
                  <span className="text-[12px] text-muted">Переключиться</span>
                )}
              </span>
              <span className="block w-full truncate font-mono text-[12px] text-faint" title={i.dir}>
                {i.dir}
              </span>
            </button>
          );
        })}
      </div>
      <div className="panel">
        <InstanceSetup build={!instances.some((i) => i.build)} />
      </div>
    </section>
  );
}

/** Versions of everything the launcher updates, and how often it looks for new ones by itself. */
function Updates() {
  const { settings, status, build, progress, launcherUpdate, updatesChecking, lastCheck, saveSettings, checkUpdates, startUpdate, setPage, run } =
    useApp();
  const [version, setVersion] = useState<string | null>(null);
  const [installing, setInstalling] = useState(false);
  useEffect(() => {
    api.launcherVersion().then(setVersion, () => {});
  }, []);
  if (!settings) return null;
  const isBuild = !!activeInstance(settings)?.build;
  const busy = !!progress || !!status?.updating;
  // Applied at once, like the author mode switch: a timer setting has nothing to review before saving.
  const setHours = (hours: number) => void saveSettings({ ...settings, updateCheckHours: hours });
  const checked = lastCheck
    ? new Date(lastCheck).toLocaleString("ru-RU", { day: "numeric", month: "long", hour: "2-digit", minute: "2-digit" })
    : "ещё не было";
  // The installer closes the launcher and starts the new version, so there is no "done" state.
  const installLauncher = () =>
    run(async () => {
      setInstalling(true);
      try {
        await api.installLauncherUpdate();
      } finally {
        setInstalling(false);
      }
    });
  const updateBuild = () => {
    setPage("mods");
    void startUpdate();
  };

  const installed = status?.installedVersion ?? null;
  const buildRow: Omit<VersionRowProps, "icon" | "title" | "hint"> = !build
    ? { current: installed, state: { tone: "muted", text: "Загрузка…" } }
    : settings.authorMode
      ? { current: installed, state: { tone: "muted", text: "Режим автора: не обновляется" } }
      : !build.online
        ? { current: installed, state: { tone: "muted", text: "Нет связи с GitHub" } }
        : build.upToDate
          ? { current: installed, state: { tone: "ok", text: "Последняя версия" } }
          : {
              current: installed,
              next: isRepairOnly(build) ? undefined : build.latestVersion,
              state: { tone: "accent", text: isRepairOnly(build) ? "Нужно восстановить файлы" : installed ? "Есть обновление" : "Не установлена" },
              detail: `${build.changes} ${plural(build.changes, "изменение", "изменения", "изменений")} · ${downloadLabel(build)}`,
              action: (
                <Button className="h-8 text-[13px]" disabled={busy} onClick={updateBuild}>
                  {installed ? "Обновить" : "Установить"}
                </Button>
              ),
            };

  return (
    <div className="panel">
      <div className="flex items-center gap-4 px-5 pt-5 pb-3">
        <div className="min-w-0 flex-1">
          <div className="text-sm font-medium">Версии</div>
          <div className="mt-0.5 text-[13px] text-muted">Последняя проверка: {checked}</div>
        </div>
        <Button onClick={() => void checkUpdates()} disabled={updatesChecking}>
          <RotateCw size={15} className={updatesChecking ? "animate-spin" : ""} />
          {updatesChecking ? "Проверка…" : "Проверить сейчас"}
        </Button>
      </div>
      <ul className="mx-5 divide-y divide-line/60 border-y border-line/60">
        <VersionRow
          icon={<AppWindow size={17} />}
          title="Лаунчер"
          hint="Это приложение"
          current={version}
          next={launcherUpdate?.version}
          state={launcherUpdate ? { tone: "accent", text: "Есть обновление" } : { tone: "ok", text: "Последняя версия" }}
          detail={launcherUpdate?.notes ?? undefined}
          action={
            launcherUpdate && (
              <Button className="h-8 text-[13px]" onClick={installLauncher} disabled={installing || busy}>
                {installing ? <Loader2 size={14} className="animate-spin" /> : <Download size={14} />}
                {installing ? "Загрузка…" : "Обновить"}
              </Button>
            )
          }
        />
        {isBuild && (
          <VersionRow icon={<Layers size={17} />} title="Сборка модов" hint={build ? `Для патча игры ${build.gameVersion}` : "LYNO//HARDWIRED"} {...buildRow} />
        )}
      </ul>
      <div className="px-5 py-5">
        <div className="text-sm font-medium">Проверять автоматически</div>
        <div className="mt-0.5 text-[13px] text-muted">
          Лаунчер и сборка: пока лаунчер открыт, а если он был закрыт дольше — сразу при запуске. Найденное появится над списком модов. Версии модов с Nexus проверяются кнопкой на странице «Моды».
        </div>
        <div className="mt-3 flex flex-wrap gap-1.5">
          {CHECK_HOURS.map(([hours, label]) => (
            <button
              key={hours}
              onClick={() => setHours(hours)}
              aria-pressed={settings.updateCheckHours === hours}
              className={`h-8 rounded-full px-3.5 text-[13px] transition-colors ${
                settings.updateCheckHours === hours ? "bg-raised text-fg ring-1 ring-neon/50" : "text-muted hover:bg-raised hover:text-fg"
              }`}
            >
              {label}
            </button>
          ))}
        </div>
      </div>
    </div>
  );
}

type Tone = "ok" | "accent" | "muted";

const toneClass: Record<Tone, string> = {
  ok: "bg-ok/10 text-ok",
  accent: "bg-accent/10 text-accent",
  muted: "bg-raised text-muted",
};

interface VersionRowProps {
  icon: ReactNode;
  title: string;
  hint: string;
  current: string | null;
  next?: string;
  state: { tone: Tone; text: string };
  detail?: string;
  action?: ReactNode;
}

function VersionRow({ icon, title, hint, current, next, state, detail, action }: VersionRowProps) {
  return (
    <li className="grid grid-cols-[minmax(0,1fr)_8rem_minmax(0,1.4fr)_7rem] items-center gap-4 py-3.5">
      <div className="flex min-w-0 items-center gap-3">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-full bg-raised text-muted">{icon}</span>
        <div className="min-w-0">
          <div className="truncate text-sm">{title}</div>
          <div className="text-[12px] text-faint">{hint}</div>
        </div>
      </div>
      <div className="font-mono text-[13px] tabular-nums">
        <span className={next ? "text-muted" : ""}>{current ?? "—"}</span>
        {next && <span className="text-accent"> → {next}</span>}
      </div>
      <div className="min-w-0">
        <span className={`inline-block rounded-full px-2.5 py-0.5 text-[12px] ${toneClass[state.tone]}`}>{state.text}</span>
        {detail && (
          <div className="mt-1 line-clamp-2 text-[12px] text-faint" title={detail}>
            {detail}
          </div>
        )}
      </div>
      <div className="flex justify-end">{action}</div>
    </li>
  );
}

const CHECK_HOURS: [number, string][] = [
  [1, "Каждый час"],
  [6, "Каждые 6 часов"],
  [24, "Раз в день"],
  [0, "Только вручную"],
];

const folders: [Folder, string][] = [
  ["saves", "Сохранения"],
  ["instance", "Mod Organizer 2"],
  ["game", "Игра"],
  ["logs", "Логи лаунчера"],
];

function Field(props: { label: string; hint: string; children: React.ReactNode }) {
  return (
    <div className="px-5 py-5">
      <div className="text-sm font-medium">{props.label}</div>
      <div className="mt-0.5 text-[13px] text-muted">{props.hint}</div>
      <div className="mt-3">{props.children}</div>
    </div>
  );
}

function Input({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <input
      value={value}
      spellCheck={false}
      onChange={(e) => onChange(e.target.value)}
      className="h-9 w-full rounded-full bg-raised px-4 font-mono text-[13px] outline-none focus:ring-1 focus:ring-neon/50"
    />
  );
}

/**
 * Author mode is for whoever can push to the build's repository, and GitHub decides that: players get no switch
 * that would quietly stop their updates.
 */
function AuthorUnlock({ repo, onEnabled }: { repo: string; onEnabled: () => void }) {
  const { run, refreshStatus } = useApp();
  const [open, setOpen] = useState(false);
  const [token, setToken] = useState("");
  const [busy, setBusy] = useState(false);

  if (!open) {
    return (
      <button onClick={() => setOpen(true)} className="text-[11px] text-faint underline-offset-2 hover:text-muted hover:underline">
        Я автор сборки
      </button>
    );
  }
  const enable = () =>
    run(async () => {
      setBusy(true);
      try {
        await api.authorEnable(token.trim() || null);
        await refreshStatus();
        onEnabled();
      } finally {
        setBusy(false);
      }
    });

  return (
    <div className="panel px-5 py-5">
      <div className="text-sm font-medium">Режим автора</div>
      <div className="mt-0.5 text-[13px] text-muted">
        Для того, кто выпускает сборку: правки модов в этой папке становятся следующим выпуском, обновление и починка
        выключаются, появляется вкладка «Выпуск». Включается токеном GitHub с правом записи в {repo || "репозиторий сборки"}{" "}
        (fine-grained token, Contents: Read and write). Токен сохранится в диспетчере учётных данных Windows.
      </div>
      <div className="mt-3 flex gap-2">
        <input
          type="password"
          value={token}
          spellCheck={false}
          autoComplete="off"
          placeholder="github_pat_…"
          onChange={(e) => setToken(e.target.value)}
          className="h-9 min-w-0 flex-1 rounded-full bg-raised px-4 font-mono text-[13px] outline-none focus:ring-1 focus:ring-neon/50"
        />
        <Button variant="primary" onClick={enable} disabled={busy || !token.trim()}>
          {busy && <Loader2 size={15} className="animate-spin" />}
          Включить
        </Button>
        <Button variant="ghost" onClick={() => setOpen(false)}>
          Отмена
        </Button>
      </div>
    </div>
  );
}
