import { ArrowRight, Download, FolderCog, FolderSearch, Loader2, Play, RefreshCw, Wrench } from "lucide-react";
import { useState, type ReactNode } from "react";
import { api } from "../api";
import { Button } from "../components/Button";
import { Changelog } from "../components/Changelog";
import { Section } from "../components/Section";
import { UpdateProgress } from "../components/UpdateProgress";
import logo from "../assets/logo.webp";
import { formatBytes } from "../format";
import { isRepairOnly, primaryAction, useApp, type PrimaryAction } from "../store";

export function Home() {
  const { status, build, buildError, progress, run, setPage, startUpdate } = useApp();
  const [launching, setLaunching] = useState(false);
  const action = primaryAction(status, build, !!progress);

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

  const showUpdateBanner = action === "update" && build;

  return (
    <div className="grid h-full grid-cols-[1fr_380px] gap-6">
      <div className="flex min-h-0 flex-col">
        <div className="text-[13px] text-muted">Cyberpunk 2077 · сборка модов</div>
        <h1 className="mt-3">
          <img src={logo} alt="LYNO//HARDWIRED" draggable={false} className="h-20 w-auto max-w-full" />
        </h1>
        <div className="mt-3 flex gap-2 font-mono text-xs text-muted tabular-nums">
          <Tag>{status?.installedVersion ? `v${status.installedVersion}` : "не установлена"}</Tag>
          {build && <Tag>патч {build.gameVersion}</Tag>}
        </div>

        {showUpdateBanner && (
          <button
            onClick={() => setPage("updates")}
            className="mt-8 flex items-center gap-3 rounded-full bg-warn/[0.07] py-2.5 pr-4 pl-5 text-left transition-colors hover:bg-warn/[0.12]"
          >
            <span className="flex-1">
              {isRepairOnly(build) ? (
                "Нужно восстановить повреждённые файлы"
              ) : (
                <>
                  Доступна версия <span className="font-mono font-semibold tabular-nums">{build.latestVersion}</span>
                </>
              )}
              <span className="text-muted"> · {formatBytes(build.downloadSize)} к загрузке</span>
            </span>
            <ArrowRight size={16} className="text-muted" />
          </button>
        )}
        {buildError && !build && (
          <div className="mt-8 flex items-center gap-3 rounded-full bg-bad/[0.07] px-5 py-2.5 text-[13px]">
            <span className="flex-1 text-bad">{buildError}</span>
          </div>
        )}

        <dl className="panel mt-6 divide-y divide-line/60 py-1">
          <Row
            label="Сборка"
            tone={!status ? "idle" : status.installedVersion ? "ok" : "warn"}
            value={!status ? "—" : status.installedVersion ? `Установлена, ${status.modsEnabled} из ${status.modsTotal} модов включено` : "Не установлена"}
          />
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
          <div className="flex items-center gap-3">
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
          </div>
        </div>
      </div>

      <Section title="Что нового" className="flex min-h-0 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto">
          {build ? (
            <Changelog entries={build.changelog} installed={build.installedVersion} />
          ) : (
            <p className="px-5 pb-4 text-[13px] text-muted">{buildError ? "Нет данных о сборке." : "Загрузка…"}</p>
          )}
        </div>
      </Section>
    </div>
  );
}

function primaryLabel(action: PrimaryAction, launching: boolean, latest: string | undefined, repair: boolean): ReactNode {
  if (launching) return <><Loader2 size={18} className="animate-spin" />Запуск…</>;
  switch (action) {
    case "loading":
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
