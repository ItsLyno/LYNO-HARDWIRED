import { ChevronDown, Download, Loader2, X } from "lucide-react";
import { useState, type ReactNode } from "react";
import { api, type GameMismatch, type LastUpdate, type OwnCopy, type RunReport } from "../api";
import { downloadLabel, plural } from "../format";
import { activeInstance, isRepairOnly, useApp } from "../store";
import { Button } from "./Button";
import { Changelog } from "./Changelog";
import { RunProblems } from "./RunProblems";
import { UpdateProgress } from "./UpdateProgress";

/** What needs the player's attention, and only that: a healthy setup shows nothing. */
export function Notices() {
  const { status, build, progress, settings, launcherUpdate, runReport, dismissRunReport, setPage, run } = useApp();
  const [problems, setProblems] = useState(false);
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
      {status?.gameMismatch && <GameMismatchNotice m={status.gameMismatch} />}
      {isBuild && build?.lastUpdate && build.lastUpdate.ownCopies.length > 0 && !settings?.authorMode && <OwnCopiesNotice update={build.lastUpdate} />}
      {runReport && (
        <Notice tone={runReport.scriptsFailed ? "bad" : "accent"} text={runSummary(runReport)}>
          <Button className="h-8 text-[13px]" onClick={() => setProblems(true)}>
            Подробнее
          </Button>
          <button onClick={() => dismissRunReport(false)} aria-label="Скрыть" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <X size={15} />
          </button>
        </Notice>
      )}
      {problems && runReport && <RunProblems report={runReport} onClose={() => setProblems(false)} />}
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

/** The game is another patch than the build: frameworks (RED4ext, CET, redscript) are made for one patch, so
 *  after a game update the build often breaks until its own update. Playing stays allowed: the player decides. */
function GameMismatchNotice({ m }: { m: GameMismatch }) {
  const [open, setOpen] = useState(false);
  const yours = m.found ? `патч ${m.found}` : "другая версия";
  const text =
    m.newer === false
      ? `Сборка сделана для патча ${m.expected}, а у вас ${yours}: обновите игру`
      : m.newer
        ? `Игра обновилась (${yours}), а сборка сделана для патча ${m.expected}`
        : `Сборка сделана для патча ${m.expected}, а у вас ${yours}`;
  return (
    <Notice
      tone="bad"
      text={text}
      below={
        open && (
          <div className="space-y-2 border-t border-line/60 px-5 py-4 text-muted">
            {m.newer === false ? (
              <p>Обновите Cyberpunk 2077 в Steam, GOG или Epic: моды сборки рассчитаны на более новый патч.</p>
            ) : (
              <>
                <p>
                  RED4ext, Cyber Engine Tweaks, redscript и моды на них работают с одним патчем игры. После обновления
                  игры они часто не запускаются, пока не выйдут их новые версии и обновление сборки.
                </p>
                <p>
                  Играть можно, но если игра падает или моды не работают — дождитесь обновления сборки. Чтобы игра не
                  обновлялась без спроса: в Steam — свойства игры → Обновления → «Обновлять игру только при её
                  запуске», в GOG Galaxy и Epic — отключите автообновление игры.
                </p>
              </>
            )}
          </div>
        )
      }
    >
      <Button variant="ghost" className="h-8 text-[13px]" onClick={() => setOpen(!open)} aria-expanded={open}>
        Подробнее
        <ChevronDown size={14} className={`transition-transform ${open ? "rotate-180" : ""}`} />
      </Button>
    </Notice>
  );
}

const OWN_COPIES_SEEN = "lyno.ownCopiesSeen";

/** The update brought mods the player had installed themselves: theirs are switched off so the build's version
 *  loads (two copies of one mod break each other). An optional one they may keep instead of the build's. */
function OwnCopiesNotice({ update }: { update: LastUpdate }) {
  const { run, refreshBuild, refreshUserMods } = useApp();
  const [open, setOpen] = useState(false);
  const [seen, setSeen] = useState(() => {
    try {
      return localStorage.getItem(OWN_COPIES_SEEN) === update.to;
    } catch {
      return false;
    }
  });
  if (seen) return null;
  const dismiss = () => {
    try {
      localStorage.setItem(OWN_COPIES_SEEN, update.to);
    } catch {
      // Shows again next time.
    }
    setSeen(true);
  };
  const keepMine = (c: OwnCopy) =>
    run(async () => {
      await api.setModEnabled(c.id, false);
      await api.setUserModEnabled(c.folder, true);
      await Promise.all([refreshBuild(), refreshUserMods()]);
    });
  const n = update.ownCopies.length;
  return (
    <Notice
      tone="accent"
      text={
        n === 1
          ? `В сборку ${update.to} вошёл мод «${update.ownCopies[0].title}», который у вас уже стоял: ваш выключен`
          : `В сборку ${update.to} вошли ${n} ${plural(n, "мод", "мода", "модов")}, которые у вас уже стояли: ваши выключены`
      }
      below={
        open && (
          <div className="space-y-3 border-t border-line/60 px-5 py-4 text-muted">
            <p>
              Две копии одного мода мешают друг другу, а версия сборки проверена вместе с остальными модами — поэтому
              работает она. Ваши копии остались в ваших модах, выключенными: их можно удалить или включить обратно.
            </p>
            <ul className="space-y-2">
              {update.ownCopies.map((c) => (
                <li key={c.folder} className="flex items-center gap-3">
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-fg">{c.folder}</div>
                    <div className="truncate text-[12px]">
                      {c.renamedFrom ? `Переименован из «${c.renamedFrom}»: это название теперь у мода сборки` : `Тот же мод на Nexus, что и «${c.title}» в сборке`}
                    </div>
                  </div>
                  {c.optional ? (
                    <Button variant="ghost" className="h-8 shrink-0 text-[13px]" title="Выключить мод сборки и включить ваш" onClick={() => keepMine(c)}>
                      Оставить мой
                    </Button>
                  ) : (
                    <span className="shrink-0 text-[12px]" title="Без него сборка не работает">
                      Основа сборки
                    </span>
                  )}
                </li>
              ))}
            </ul>
          </div>
        )
      }
    >
      <Button variant="ghost" className="h-8 text-[13px]" onClick={() => setOpen(!open)} aria-expanded={open}>
        Подробнее
        <ChevronDown size={14} className={`transition-transform ${open ? "rotate-180" : ""}`} />
      </Button>
      <button onClick={dismiss} aria-label="Скрыть" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
        <X size={15} />
      </button>
    </Notice>
  );
}

function runSummary(r: RunReport): string {
  const mods = [...new Set(r.problems.flatMap((p) => (p.modFolder ? [p.modFolder] : [])))];
  const named = mods.length === 0 ? "" : mods.length === 1 ? `: «${mods[0]}»` : `: «${mods[0]}» и ещё ${mods.length - 1}`;
  if (r.scriptsFailed) return `Скрипты модов не собрались, моды на redscript не работали${named}`;
  const n = r.problems.filter((p) => p.severity === "error").reduce((sum, p) => sum + p.count, 0);
  if (n === 0) return `Плагины не загрузились или не подходят к версии игры${named}`;
  return `В последнем запуске моды записали ${n} ${plural(n, "ошибку", "ошибки", "ошибок")}${named}`;
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
