import { Check, FileArchive, FolderOpen, Loader2, Search } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type Folder, type Settings as SettingsT } from "../api";
import { Button } from "../components/Button";
import { Integrity } from "../components/Integrity";
import { PageTitle } from "../components/PageTitle";
import { useApp } from "../store";

export function Settings() {
  const { settings, saveSettings, run, setPage } = useApp();
  const [draft, setDraft] = useState<SettingsT | null>(settings);
  const [saved, setSaved] = useState(false);
  const [detectNote, setDetectNote] = useState<string | null>(null);
  const [version, setVersion] = useState<string | null>(null);
  const [report, setReport] = useState<{ busy: boolean; path: string | null }>({ busy: false, path: null });

  useEffect(() => {
    if (settings && !draft) setDraft(settings);
  }, [settings, draft]);
  useEffect(() => {
    api.launcherVersion().then(setVersion, () => {});
  }, []);
  if (!draft) return null;

  const dirty = JSON.stringify(draft) !== JSON.stringify(settings);
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
        <Field label="Папка сборки" hint="Сюда устанавливаются Mod Organizer 2 и моды. Нужно около 1,2× размера сборки свободного места.">
          <Input value={draft.instanceDir} onChange={(v) => update({ instanceDir: v })} />
        </Field>
        <Field label="Манифест сборки" hint="Адрес файла с описанием актуальной версии сборки на GitHub.">
          <Input value={draft.manifestUrl} onChange={(v) => update({ manifestUrl: v })} />
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
            if (await saveSettings(draft)) setSaved(true);
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
        {settings?.authorMode ? (
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
        LYNO//HARDWIRED {version ?? ""} · Моды принадлежат их авторам.
      </p>
      {!settings?.authorMode && <AuthorUnlock repo={settings?.authorRepo ?? ""} onEnabled={authorEnabled} />}
    </div>
  );
}

const folders: [Folder, string][] = [
  ["saves", "Сохранения"],
  ["instance", "Сборка"],
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
