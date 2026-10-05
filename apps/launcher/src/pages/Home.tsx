import { ChevronUp, Download, FolderCog, FolderSearch, Loader2, Play, RefreshCw, RotateCw, Wrench } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { api, type Executable } from "../api";
import { Button } from "../components/Button";
import { Changelog } from "../components/Changelog";
import { InstanceSetup } from "../components/InstanceSetup";
import { Section } from "../components/Section";
import { UpdateProgress } from "../components/UpdateProgress";
import logo from "../assets/logo.webp";
import { downloadLabel, plural } from "../format";
import { activeInstance, isRepairOnly, primaryAction, useApp, type PrimaryAction } from "../store";

export function Home() {
  const { status, build, buildError, progress, settings, run, setPage, startUpdate, switchInstance } = useApp();
  const [launching, setLaunching] = useState(false);
  const action = primaryAction(status, build, !!progress, settings);
  const instance = activeInstance(settings);
  // Before the first setup the build is offered as one of the choices too.
  const isBuild = instance ? instance.build : false;

  const onPrimary = async () => {
    switch (action) {
      case "play":
        setLaunching(true);
        await run(api.launchGame);
        // MO2 deploys REDmod before the game window appears; keep the button busy meanwhile.
        setTimeout(() => setLaunching(false), 5000);
        return;
      case "install":
      case "update":
        return startUpdate();
      case "game":
        return setPage("settings");
    }
  };

  return (
    <div className="grid h-full grid-cols-[1fr_380px] gap-6">
      <div className="flex min-h-0 flex-col">
        <div className="text-[13px] text-muted">Cyberpunk 2077 · {isBuild ? "сборка модов" : "Mod Organizer 2"}</div>
        <h1 className="mt-3">
          <img src={logo} alt="LYNO//HARDWIRED" draggable={false} className="h-20 w-auto max-w-full" />
        </h1>
        <div className="mt-3 flex gap-2 font-mono text-xs text-muted tabular-nums">
          {isBuild && <Tag>{status?.installedVersion ? `v${status.installedVersion}` : "не установлена"}</Tag>}
          {build && <Tag>патч {build.gameVersion}</Tag>}
          {instance && !isBuild && <Tag>{instance.name}</Tag>}
        </div>

        {buildError && !build && (
          <div className="mt-8 flex items-center gap-3 rounded-full bg-bad/[0.07] px-5 py-2.5 text-[13px]">
            <span className="flex-1 text-bad">{buildError}</span>
          </div>
        )}

        {action === "setup" && (
          <div className="panel mt-6">
            <InstanceSetup build={false} />
          </div>
        )}
        <dl className={`panel mt-6 divide-y divide-line/60 py-1 ${action === "setup" ? "hidden" : ""}`}>
          {isBuild ? (
            <Row
              label="Сборка"
              tone={!status ? "idle" : status.installedVersion ? "ok" : "warn"}
              value={!status ? "—" : status.installedVersion ? `Установлена, ${status.modsEnabled} из ${status.modsTotal} модов включено` : "Не установлена"}
            />
          ) : (
            <Row
              label="Mod Organizer 2"
              tone={!status ? "idle" : status.mo2Installed ? "ok" : "bad"}
              value={!status ? "—" : status.mo2Installed ? `${status.modsEnabled} из ${status.modsTotal} модов включено` : "Не найден"}
            />
          )}
          <Row
            label="Cyberpunk 2077"
            tone={!status ? "idle" : status.gameFound ? "ok" : "bad"}
            value={!status ? "—" : status.gameFound ? (status.gameDir ?? "") : "Папка игры не найдена"}
          />
          <Row
            label="Состояние"
            tone={status?.gameRunning ? "ok" : "idle"}
            value={status?.gameRunning ? "Игра запущена" : status?.mo2Running ? "Открыт Mod Organizer 2" : "Готово к запуску"}
          />
        </dl>

        <div className="mt-auto space-y-4 pt-8">
          {progress && <UpdateProgress progress={progress} />}
          <div className={`flex items-center gap-3 ${action === "setup" ? "hidden" : ""}`}>
            <Button
              variant="primary"
              size="lg"
              className="min-w-56"
              disabled={action === "loading" || action === "running" || action === "updating" || launching}
              onClick={onPrimary}
            >
              {primaryLabel(action, launching, build?.latestVersion, isRepairOnly(build))}
            </Button>
            <Button size="lg" disabled={!status?.mo2Installed || !!progress} onClick={() => run(api.openMo2)}>
              <FolderCog size={17} />
              Открыть MO2
            </Button>
            <ToolsMenu disabled={!status?.mo2Installed || !!progress} />
          </div>
        </div>
      </div>

      <div className="flex min-h-0 flex-col gap-6">
        <UpdatesPanel isBuild={isBuild} />
        {isBuild ? (
          <Section title="Что нового" className="flex min-h-0 flex-1 flex-col">
            <div className="min-h-0 flex-1 overflow-y-auto">
              {build ? (
                <Changelog entries={build.changelog} installed={build.installedVersion} />
              ) : (
                <p className="px-5 pb-4 text-[13px] text-muted">{buildError ? "Нет данных о сборке." : "Загрузка…"}</p>
              )}
            </div>
          </Section>
        ) : (
          <Section title="Сборка LYNO//HARDWIRED" className="self-start">
            <div className="px-5 pb-5 text-[13px] text-muted">
              <p>
                Готовая сборка модов с обновлениями в один клик. Ставится в отдельную папку со своим Mod Organizer 2 — ваш
                MO2 и его моды она не трогает. Переключаться между ними можно в настройках.
              </p>
              <Button className="mt-4" disabled={!!progress} onClick={() => switchInstance(api.instanceAddBuild)}>
                <Download size={15} />
                Перейти к сборке
              </Button>
            </div>
          </Section>
        )}
      </div>
    </div>
  );
}

/** MO2's other executables (REDmod, WolvenKit, what the player added in MO2), started inside its virtual file system. */
function ToolsMenu({ disabled }: { disabled: boolean }) {
  const { run, settings } = useApp();
  const [tools, setTools] = useState<Executable[]>([]);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    api.executables().then(setTools, () => setTools([]));
  }, [settings?.instanceDir]);
  useEffect(() => {
    if (!open) return;
    const outside = (e: Event) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", outside);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", outside);
      window.removeEventListener("keydown", esc);
    };
  }, [open]);
  if (tools.length === 0) return null;
  return (
    <div ref={ref} className="relative">
      <Button size="lg" disabled={disabled} onClick={() => setOpen(!open)} aria-haspopup="menu" aria-expanded={open}>
        <Wrench size={16} />
        Программы
        <ChevronUp size={14} className={`transition-transform ${open ? "" : "rotate-180"}`} />
      </Button>
      {open && (
        <div role="menu" className="panel absolute bottom-full left-0 z-50 mb-2 w-72 bg-surface py-1.5 shadow-2xl">
          {tools.map((t) => (
            <button
              key={t.title}
              role="menuitem"
              title={t.binary}
              onClick={() => {
                setOpen(false);
                void run(() => api.launchExecutable(t.title));
              }}
              className="flex h-8 w-full items-center gap-2.5 px-3.5 text-left text-[13px] text-fg outline-none hover:bg-raised focus:bg-raised"
            >
              <Play size={13} className="shrink-0 opacity-70" />
              <span className="truncate">{t.title}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

/** Build and launcher versions side by side: one "check" asks GitHub about both. */
function UpdatesPanel({ isBuild }: { isBuild: boolean }) {
  const { build, buildError, status, progress, settings, launcherUpdate, refreshBuild, checkLauncherUpdate, run } = useApp();
  const [version, setVersion] = useState<string | null>(null);
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  useEffect(() => {
    api.launcherVersion().then(setVersion, () => {});
  }, []);
  const busy = !!progress || !!status?.updating;
  const installed = status?.installedVersion ?? null;

  const check = async () => {
    setChecking(true);
    await Promise.all([isBuild ? refreshBuild() : null, checkLauncherUpdate()]);
    setChecking(false);
  };
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

  let buildLine: ReactNode = null;
  if (isBuild) {
    buildLine = !build ? (
      <span className="text-muted">{buildError ?? "Загрузка…"}</span>
    ) : !build.online ? (
      <span className="text-muted">нет связи с GitHub</span>
    ) : build.upToDate ? (
      <span className="text-muted">последняя</span>
    ) : settings?.authorMode ? (
      <span className="text-muted">режим автора: обновление выключено</span>
    ) : (
      <span className="text-accent">
        {isRepairOnly(build) ? "нужно восстановить файлы" : installed ? `доступна ${build.latestVersion}` : "не установлена"}
        <span className="text-muted">
          {" "}· {build.changes} {plural(build.changes, "изменение", "изменения", "изменений")} · {downloadLabel(build)}
        </span>
      </span>
    );
  }

  return (
    <Section
      title="Обновления"
      action={
        <Button variant="ghost" className="-mr-2 h-8 text-[13px]" onClick={check} disabled={checking || busy}>
          <RotateCw size={14} className={checking ? "animate-spin" : ""} />
          Проверить
        </Button>
      }
    >
      <dl className="divide-y divide-line/60 border-t border-line/60 text-[13px]">
        {isBuild && (
          <div className="px-5 py-3">
            <dt className="label">Сборка</dt>
            <dd className="mt-1">
              <span className="font-mono tabular-nums">{installed ?? "—"}</span>
              {build && <span className="text-muted"> · патч {build.gameVersion}</span>}
              <div className="mt-0.5">{buildLine}</div>
            </dd>
          </div>
        )}
        <div className="flex items-center gap-3 px-5 py-3">
          <div className="min-w-0 flex-1">
            <dt className="label">Лаунчер</dt>
            <dd className="mt-1">
              <span className="font-mono tabular-nums">{version ?? "—"}</span>
              <div className="mt-0.5 truncate" title={launcherUpdate?.notes ?? undefined}>
                {launcherUpdate ? (
                  <span className="text-accent">доступна {launcherUpdate.version}</span>
                ) : (
                  <span className="text-muted">последняя</span>
                )}
              </div>
            </dd>
          </div>
          {launcherUpdate && (
            <Button
              className="h-8 text-[13px]"
              onClick={installLauncher}
              disabled={installing || busy}
              title={busy ? "Дождитесь окончания обновления сборки" : undefined}
            >
              {installing ? <Loader2 size={14} className="animate-spin" /> : <Download size={14} />}
              {installing ? "Загрузка…" : "Обновить"}
            </Button>
          )}
        </div>
      </dl>
    </Section>
  );
}

function primaryLabel(action: PrimaryAction, launching: boolean, latest: string | undefined, repair: boolean): ReactNode {
  if (launching) return <><Loader2 size={18} className="animate-spin" />Запуск…</>;
  switch (action) {
    case "loading":
    case "setup":
      return <Loader2 size={18} className="animate-spin" />;
    case "updating":
      return <><Loader2 size={18} className="animate-spin" />Обновление…</>;
    case "running":
      return "Игра запущена";
    case "game":
      return <><FolderSearch size={18} />Указать папку игры</>;
    case "install":
      return <><Download size={18} />Установить сборку</>;
    case "update":
      return repair ? <><Wrench size={18} />Восстановить файлы</> : <><RefreshCw size={18} />Обновить до {latest}</>;
    case "play":
      return <><Play size={18} fill="currentColor" />Играть</>;
  }
}

type Tone = "ok" | "warn" | "bad" | "idle";

// Only problems get a color; a healthy row stays plain.
const toneText: Record<Tone, string> = { ok: "", idle: "", warn: "text-warn", bad: "text-bad" };

function Row({ label, value, tone }: { label: string; value: string; tone: Tone }) {
  return (
    <div className="flex h-11 items-center gap-3 px-5">
      <dt className="label w-40 shrink-0">{label}</dt>
      <dd className={`truncate ${toneText[tone]}`} title={value}>
        {value}
      </dd>
    </div>
  );
}

function Tag({ children }: { children: ReactNode }) {
  return <span className="rounded-full bg-raised px-2.5 py-0.5">{children}</span>;
}
