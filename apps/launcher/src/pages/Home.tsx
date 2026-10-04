import { ArrowRight, FolderCog, Loader2, Play, Download, RefreshCw, Settings2 } from "lucide-react";
import { useState, type ReactNode } from "react";
import { api } from "../api";
import { Button } from "../components/Button";
import { Changelog } from "../components/Changelog";
import { Section } from "../components/Section";
import { StatusDot, type Tone } from "../components/StatusDot";
import { primaryAction, useApp } from "../store";

export function Home() {
  const { status, settings, build, run, setPage } = useApp();
  const [launching, setLaunching] = useState(false);
  const action = primaryAction(status, build);
  const updateAvailable = action === "update";

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
        return setPage("build");
      case "setup":
        return setPage("settings");
    }
  };

  return (
    <div className="grid h-full grid-cols-[1fr_380px] gap-6">
      <div className="flex min-h-0 flex-col">
        <div className="text-[13px] text-muted">Cyberpunk 2077 · сборка модов</div>
        <h1 className="mt-1 text-[40px] leading-tight font-semibold tracking-tight">
          LYNO<span className="text-faint">//</span>HARDWIRED
        </h1>
        <div className="mt-2 flex gap-4 text-[13px] text-muted tabular-nums">
          <span>Версия {build?.installedVersion ?? "—"}</span>
          {build && <span>Патч игры {build.gameVersion}</span>}
        </div>

        {updateAvailable && build && (
          <button
            onClick={() => setPage("build")}
            className="mt-6 flex items-center gap-3 rounded-lg border border-line bg-surface px-4 py-3 text-left transition-colors hover:bg-raised"
          >
            <StatusDot tone="warn" />
            <span className="flex-1">
              Доступна версия <span className="font-semibold tabular-nums">{build.latestVersion}</span>
              <span className="text-muted"> — {build.changelog[0]?.notes.length ?? 0} изменения</span>
            </span>
            <ArrowRight size={16} className="text-muted" />
          </button>
        )}

        <dl className="mt-6 divide-y divide-line rounded-lg border border-line bg-surface">
          <Row label="Mod Organizer 2" {...mo2Row(status)} />
          <Row
            label="Профиль"
            tone={!status ? "idle" : status.profileExists ? "ok" : "warn"}
            value={settings ? (status?.profileExists ? settings.profile : `${settings.profile} — не найден`) : "—"}
          />
          <Row
            label="Моды"
            tone={status && status.modsTotal > 0 ? "ok" : "idle"}
            value={status ? `${status.modsEnabled} из ${status.modsTotal} включено` : "—"}
          />
          <Row
            label="Игра"
            tone={status?.gameRunning ? "ok" : "idle"}
            value={status?.gameRunning ? "Запущена" : "Не запущена"}
          />
        </dl>

        <div className="mt-auto flex items-center gap-3 pt-6">
          <Button
            variant="primary"
            size="lg"
            className="min-w-52"
            disabled={action === "loading" || action === "running" || launching}
            onClick={onPrimary}
          >
            {primaryLabel(action, launching, build?.latestVersion)}
          </Button>
          <Button size="lg" disabled={!status?.mo2Installed} onClick={() => run(api.openMo2)}>
            <FolderCog size={17} />
            Открыть MO2
          </Button>
        </div>
      </div>

      <Section title="Что нового" className="flex min-h-0 flex-col">
        <div className="min-h-0 flex-1 overflow-y-auto">
          {build ? (
            <Changelog entries={build.changelog} installed={build.installedVersion} />
          ) : (
            <p className="px-5 py-4 text-[13px] text-muted">
              Список изменений появится, когда лаунчер загрузит манифест сборки.
            </p>
          )}
        </div>
      </Section>
    </div>
  );
}

function primaryLabel(action: ReturnType<typeof primaryAction>, launching: boolean, latest?: string): ReactNode {
  if (launching) return <><Loader2 size={18} className="animate-spin" />Запуск…</>;
  switch (action) {
    case "loading":
      return <Loader2 size={18} className="animate-spin" />;
    case "running":
      return "Игра запущена";
    case "setup":
      return <><Settings2 size={18} />Настроить</>;
    case "install":
      return <><Download size={18} />Установить сборку</>;
    case "update":
      return <><RefreshCw size={18} />Обновить до {latest}</>;
    case "play":
      return <><Play size={18} fill="currentColor" />Играть</>;
  }
}

function mo2Row(status: ReturnType<typeof useApp.getState>["status"]): { tone: Tone; value: string } {
  if (!status) return { tone: "idle", value: "—" };
  if (!status.mo2Installed) return { tone: "bad", value: "Не найден" };
  if (!status.portable) return { tone: "warn", value: "Найден, но не портативный" };
  return { tone: "ok", value: status.mo2Running ? "Открыт" : "Готов" };
}

function Row({ label, value, tone }: { label: string; value: string; tone: Tone }) {
  return (
    <div className="flex h-11 items-center gap-3 px-4">
      <StatusDot tone={tone} />
      <dt className="w-40 text-muted">{label}</dt>
      <dd className="truncate">{value}</dd>
    </div>
  );
}
