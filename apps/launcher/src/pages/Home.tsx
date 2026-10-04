import { ArrowRight, Download, FolderCog, FolderSearch, Loader2, Play, RefreshCw } from "lucide-react";
import { useState, type ReactNode } from "react";
import { api } from "../api";
import { Button } from "../components/Button";
import { Changelog } from "../components/Changelog";
import { Section } from "../components/Section";
import { StatusDot, type Tone } from "../components/StatusDot";
import { UpdateProgress } from "../components/UpdateProgress";
import logo from "../assets/logo.webp";
import { formatBytes } from "../format";
import { primaryAction, useApp, type PrimaryAction } from "../store";

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
        <div className="mt-2 flex gap-4 text-[13px] text-muted tabular-nums">
          <span>{status?.installedVersion ? `Версия ${status.installedVersion}` : "Не установлена"}</span>
          {build && <span>Патч игры {build.gameVersion}</span>}
        </div>

        {showUpdateBanner && (
          <button
            onClick={() => setPage("updates")}
            className="mt-6 flex items-center gap-3 rounded-lg border border-line bg-surface px-4 py-3 text-left transition-colors hover:bg-raised"
          >
            <StatusDot tone="warn" />
            <span className="flex-1">
              Доступна версия <span className="font-semibold tabular-nums">{build.latestVersion}</span>
              <span className="text-muted"> · {formatBytes(build.downloadSize)} к загрузке</span>
            </span>
            <ArrowRight size={16} className="text-muted" />
          </button>
        )}
        {buildError && !build && (
          <div className="mt-6 flex items-center gap-3 rounded-lg border border-line bg-surface px-4 py-3 text-[13px]">
            <StatusDot tone="bad" />
            <span className="flex-1 text-muted">{buildError}</span>
          </div>
        )}

        <dl className="mt-6 divide-y divide-line rounded-lg border border-line bg-surface">
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

        <div className="mt-auto space-y-4 pt-6">
          {progress && <UpdateProgress progress={progress} />}
          <div className="flex items-center gap-3">
            <Button
              variant="primary"
              size="lg"
              className="min-w-52"
              disabled={action === "loading" || action === "running" || action === "updating" || launching}
              onClick={onPrimary}
            >
              {primaryLabel(action, launching, build?.latestVersion)}
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
            <p className="px-5 py-4 text-[13px] text-muted">{buildError ? "Нет данных о сборке." : "Загрузка…"}</p>
          )}
        </div>
      </Section>
    </div>
  );
}

function primaryLabel(action: PrimaryAction, launching: boolean, latest?: string): ReactNode {
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
      return <><RefreshCw size={18} />Обновить до {latest}</>;
    case "play":
      return <><Play size={18} fill="currentColor" />Играть</>;
  }
}

function Row({ label, value, tone }: { label: string; value: string; tone: Tone }) {
  return (
    <div className="flex h-11 items-center gap-3 px-4">
      <StatusDot tone={tone} />
      <dt className="w-36 shrink-0 text-muted">{label}</dt>
      <dd className="truncate" title={value}>
        {value}
      </dd>
    </div>
  );
}
