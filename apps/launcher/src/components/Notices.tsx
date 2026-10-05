import { ChevronDown, Download, Loader2 } from "lucide-react";
import { useState, type ReactNode } from "react";
import { api } from "../api";
import { downloadLabel, plural } from "../format";
import { activeInstance, isRepairOnly, useApp } from "../store";
import { Button } from "./Button";
import { Changelog } from "./Changelog";
import { UpdateProgress } from "./UpdateProgress";

/** What needs the player's attention, and only that: a healthy setup shows nothing. */
export function Notices() {
  const { status, build, progress, settings, launcherUpdate, setPage, run } = useApp();
  const [changelog, setChangelog] = useState(false);
  const [installing, setInstalling] = useState(false);
  const isBuild = !!activeInstance(settings)?.build;
  const busy = !!progress || !!status?.updating;
  const buildUpdate = isBuild && build && build.online && !build.upToDate && !settings?.authorMode ? build : null;
  // Newest first: what came after the installed version is what the update brings.
  const at = build ? build.changelog.findIndex((e) => e.version === build.installedVersion) : -1;
  const upcoming = !build ? [] : at === -1 ? build.changelog : build.changelog.slice(0, at);

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

  return (
    <div className="space-y-3 empty:hidden [&:not(:empty)]:mt-4">
      {progress && <UpdateProgress progress={progress} />}
      {status && !status.gameFound && (
        <Notice tone="bad" text="Папка Cyberpunk 2077 не найдена: без неё игру не запустить.">
          <Button className="h-8 text-[13px]" onClick={() => setPage("settings")}>
            Указать в настройках
          </Button>
        </Notice>
      )}
      {buildUpdate && !progress && (
        <Notice
          tone="accent"
          text={
            <>
              {isRepairOnly(buildUpdate)
                ? "Нужно восстановить файлы сборки"
                : buildUpdate.installedVersion
                  ? `Доступна сборка ${buildUpdate.latestVersion}`
                  : "Сборка не установлена"}
              <span className="text-muted">
                {" "}· {buildUpdate.changes} {plural(buildUpdate.changes, "изменение", "изменения", "изменений")} · {downloadLabel(buildUpdate)}
              </span>
            </>
          }
          below={
            changelog && (
              <div className="max-h-64 overflow-y-auto border-t border-line/60 pt-4">
                <Changelog entries={upcoming} installed={buildUpdate.installedVersion} />
              </div>
            )
          }
        >
          {upcoming.length > 0 && (
            <Button variant="ghost" className="h-8 text-[13px]" onClick={() => setChangelog(!changelog)} aria-expanded={changelog}>
              Что нового
              <ChevronDown size={14} className={`transition-transform ${changelog ? "rotate-180" : ""}`} />
            </Button>
          )}
        </Notice>
      )}
      {launcherUpdate && (
        <Notice tone="accent" text={`Доступен лаунчер ${launcherUpdate.version}`}>
          <Button
            className="h-8 text-[13px]"
            onClick={installLauncher}
            disabled={installing || busy}
            title={busy ? "Дождитесь окончания обновления сборки" : (launcherUpdate.notes ?? undefined)}
          >
            {installing ? <Loader2 size={14} className="animate-spin" /> : <Download size={14} />}
            {installing ? "Загрузка…" : "Обновить"}
          </Button>
        </Notice>
      )}
    </div>
  );
}

const tones = { accent: "text-accent", bad: "text-bad" };

function Notice(props: { tone: keyof typeof tones; text: ReactNode; below?: ReactNode; children?: ReactNode }) {
  return (
    <div className="panel text-[13px]">
      <div className="flex min-h-12 items-center gap-3 px-5 py-2">
        <span className={`min-w-0 flex-1 truncate ${tones[props.tone]}`}>{props.text}</span>
        {props.children}
      </div>
      {props.below}
    </div>
  );
}
