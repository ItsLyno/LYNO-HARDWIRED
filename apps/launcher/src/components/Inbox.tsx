import { Bell, Bug, ChevronDown, Copy, X, type LucideIcon } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { api, type LastUpdate, type OwnCopy, type RunReport } from "../api";
import { plural } from "../format";
import { activeInstance, useApp } from "../store";
import { Button } from "./Button";
import { RunProblems } from "./RunProblems";

const OWN_COPIES_SEEN = "lyno.ownCopiesSeen";

/** What can wait for the player to look: the last run's errors, what an update did to their mods. Kept behind a
 *  bell so the mod list stays clear; what stops the game is on the page (`Notices`), updates are footer badges. */
export function Inbox() {
  const { build, settings, runReport, dismissRunReport } = useApp();
  const [open, setOpen] = useState(false);
  const [problems, setProblems] = useState(false);
  const [ownSeen, setOwnSeen] = useState<string | null>(() => {
    try {
      return localStorage.getItem(OWN_COPIES_SEEN);
    } catch {
      return null;
    }
  });
  const ref = useRef<HTMLDivElement>(null);
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

  const isBuild = !!activeInstance(settings)?.build;
  const last = build?.lastUpdate;
  const own = isBuild && last && last.ownCopies.length > 0 && !settings?.authorMode && ownSeen !== last.to ? last : null;
  const count = [runReport, own].filter(Boolean).length;

  const dismissOwn = (to: string) => {
    try {
      localStorage.setItem(OWN_COPIES_SEEN, to);
    } catch {
      // Shows again next time.
    }
    setOwnSeen(to);
  };

  return (
    <>
      <div ref={ref} className="relative">
        <button
          onClick={() => setOpen(!open)}
          aria-label={count > 0 ? `Уведомления: ${count}` : "Уведомления"}
          aria-expanded={open}
          className={`relative flex size-9 items-center justify-center rounded-full transition-colors ${open ? "bg-raised text-fg" : "text-muted hover:bg-raised hover:text-fg"}`}
        >
          <Bell size={16} />
          {count > 0 && (
            <span
              className={`absolute -top-0.5 -right-0.5 flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px] font-semibold tabular-nums ${
                runReport?.scriptsFailed ? "bg-bad text-white" : "bg-accent text-accent-fg"
              }`}
            >
              {count}
            </span>
          )}
        </button>
        {open && (
          <div className="panel absolute top-full right-0 z-50 mt-2 max-h-[70vh] w-[420px] overflow-y-auto bg-surface text-[13px] shadow-2xl">
            <div className="px-5 pt-4 pb-2 font-medium">Уведомления</div>
            {count === 0 && <div className="px-5 pb-5 text-muted">Нового нет</div>}
            <div className="divide-y divide-line/60">
              {runReport && (
                <Item
                  tone={runReport.scriptsFailed ? "text-bad" : "text-warn"}
                  icon={Bug}
                  title={runReport.scriptsFailed ? "Скрипты модов не собрались" : "Ошибки в последнем запуске"}
                  text={runSummary(runReport)}
                  onDismiss={() => dismissRunReport(false)}
                >
                  <Button
                    className="h-8 text-[13px]"
                    onClick={() => {
                      setOpen(false);
                      setProblems(true);
                    }}
                  >
                    Подробнее
                  </Button>
                </Item>
              )}
              {own && <OwnCopiesItem update={own} onDismiss={() => dismissOwn(own.to)} />}
            </div>
          </div>
        )}
      </div>
      {problems && runReport && <RunProblems report={runReport} onClose={() => setProblems(false)} />}
    </>
  );
}

/** The update brought mods the player had installed themselves: theirs are switched off so the build's version
 *  loads (two copies of one mod break each other). An optional one they may keep instead of the build's. */
function OwnCopiesItem({ update, onDismiss }: { update: LastUpdate; onDismiss: () => void }) {
  const { run, refreshBuild, refreshUserMods } = useApp();
  const [open, setOpen] = useState(false);
  const keepMine = (c: OwnCopy) =>
    run(async () => {
      await api.setModEnabled(c.id, false);
      await api.setUserModEnabled(c.folder, true);
      await Promise.all([refreshBuild(), refreshUserMods()]);
    });
  const n = update.ownCopies.length;
  return (
    <Item
      tone="text-muted"
      icon={Copy}
      title={n === 1 ? "Ваша копия мода выключена" : "Ваши копии модов выключены"}
      text={
        n === 1
          ? `Мод «${update.ownCopies[0].title}» вошёл в сборку ${update.to}`
          : `${n} ${plural(n, "мод вошёл", "мода вошли", "модов вошли")} в сборку ${update.to}`
      }
      onDismiss={onDismiss}
      below={
        open && (
          <div className="mt-3 space-y-3 border-t border-line/60 pt-3 text-muted">
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
    </Item>
  );
}

function runSummary(r: RunReport): string {
  const mods = [...new Set(r.problems.flatMap((p) => (p.modFolder ? [p.modFolder] : [])))];
  const named = mods.length === 0 ? "" : mods.length === 1 ? `: «${mods[0]}»` : `: «${mods[0]}» и ещё ${mods.length - 1}`;
  if (r.scriptsFailed) return `Моды на redscript не работали${named}`;
  const n = r.problems.filter((p) => p.severity === "error").reduce((sum, p) => sum + p.count, 0);
  if (n === 0) return `Плагины не загрузились или не подходят к версии игры${named}`;
  return `Моды записали ${n} ${plural(n, "ошибку", "ошибки", "ошибок")}${named}`;
}

/** One entry: the icon's colour says how urgent, the title what happened, actions under the text (the list is narrow). */
function Item(props: {
  tone: string;
  icon: LucideIcon;
  title: string;
  text: string;
  onDismiss?: () => void;
  below?: ReactNode;
  children?: ReactNode;
}) {
  return (
    <div className="flex gap-3 px-5 py-4">
      <props.icon size={16} className={`mt-0.5 shrink-0 ${props.tone}`} />
      <div className="min-w-0 flex-1">
        <div className="flex items-start gap-2">
          <div className="min-w-0 flex-1">
            <div className="font-medium text-fg">{props.title}</div>
            <div className="text-muted">{props.text}</div>
          </div>
          {props.onDismiss && (
            <button onClick={props.onDismiss} aria-label="Скрыть" className="-mt-1 -mr-2 inline-flex size-7 shrink-0 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
              <X size={14} />
            </button>
          )}
        </div>
        <div className="mt-2.5 flex flex-wrap gap-2">{props.children}</div>
        {props.below}
      </div>
    </div>
  );
}
