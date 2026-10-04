import { ArrowLeft, ArrowRight, Check, History, Loader2, Lock, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type FomodSelection, type FomodState, type FomodWizard as Wizard, type GroupKind, type PluginKind } from "../api";
import { useApp } from "../store";
import { Button } from "./Button";

const groupHints: Record<GroupKind, string> = {
  exactlyOne: "один вариант",
  atMostOne: "не больше одного",
  atLeastOne: "хотя бы один",
  all: "всё обязательно",
  any: "любые",
};

const kindHints: Partial<Record<PluginKind, string>> = {
  required: "обязательно",
  recommended: "рекомендуется",
  notUsable: "недоступно",
};

/**
 * The options of a FOMOD installer, page by page like in MO2. The launcher
 * evaluates every click (visible pages, option types, group rules), so what
 * the wizard shows is what gets installed.
 */
export function FomodWizard({ jobId, onClose }: { jobId: number; onClose: () => void }) {
  const job = useApp((s) => s.nexusJobs.find((j) => j.id === jobId));
  const { run } = useApp();
  const [wizard, setWizard] = useState<Wizard | null>(null);
  const [state, setState] = useState<FomodState | null>(null);
  /** What the player touched or saw; null steps follow the installer's defaults. */
  const [picked, setPicked] = useState<FomodSelection>([]);
  const [at, setAt] = useState(0);
  const [focus, setFocus] = useState<{ group: number; plugin: number } | null>(null);
  const [busy, setBusy] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    api
      .nexusFomod(jobId)
      .then((w) => {
        setWizard(w);
        setState(w.state);
        setPicked(w.previous ?? w.outline.steps.map(() => null));
        const first = w.state.visible.indexOf(true);
        setAt(first < 0 ? 0 : first);
      })
      .catch((e) => setLoadError(String(e)));
  }, [jobId]);

  // The job left "choosing" (cancelled, or installing from another window): nothing to choose anymore.
  useEffect(() => {
    if (job && job.state.kind !== "choosing") onClose();
  }, [job?.state.kind]);

  const steps = wizard?.outline.steps ?? [];
  const visibleSteps = state ? steps.map((_, i) => i).filter((i) => state.visible[i]) : [];
  const pos = visibleSteps.indexOf(at);
  const last = pos === visibleSteps.length - 1 || visibleSteps.length === 0;

  const evaluate = (next: FomodSelection) =>
    run(async () => {
      setPicked(next);
      setState(await api.nexusFomodEval(jobId, next));
    });

  /** Seen steps keep what they showed, so later flags can't silently change them. */
  const seen = (s: FomodState, i: number): FomodSelection => picked.map((p, j) => (j === i && p === null ? s.selection[i] : p));

  const click = (group: number, plugin: number) => {
    if (!state || !wizard) return;
    const kind = state.kinds[at][group][plugin];
    if (kind === "required" || kind === "notUsable") return;
    const groupKind = steps[at].groups[group].kind;
    const current = state.selection[at];
    const has = current[group].includes(plugin);
    let options: number[];
    switch (groupKind) {
      case "all":
        return;
      case "exactlyOne":
        options = [plugin];
        break;
      case "atMostOne":
        options = has ? [] : [plugin];
        break;
      default:
        options = has ? current[group].filter((p) => p !== plugin) : [...current[group], plugin];
    }
    const step = current.map((g, i) => (i === group ? options : g));
    void evaluate(picked.map((p, i) => (i === at ? step : p)));
  };

  const go = (to: number) => {
    if (!state) return;
    setPicked(seen(state, at));
    setAt(to);
    setFocus(null);
  };

  // Exactly what the wizard shows: every visible step as evaluated, hidden ones empty.
  const install = (selection: FomodSelection) =>
    run(async () => {
      setBusy(true);
      try {
        await api.nexusFomodInstall(jobId, selection);
        onClose();
      } finally {
        setBusy(false);
      }
    });
  const shown = (s: FomodState): FomodSelection => s.selection.map((sel, i) => (s.visible[i] ? sel : null));

  const step = steps[at];
  const focused = step && state ? (focus ?? firstPicked(state.selection[at])) : null;
  const plugin = focused && step ? step.groups[focused.group]?.plugins[focused.plugin] : null;
  const image = (plugin?.image && wizard?.images[plugin.image]) || (wizard?.outline.image && wizard.images[wizard.outline.image]) || null;
  const title = wizard?.outline.name ?? job?.title ?? "Установщик мода";

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-8" role="dialog" aria-modal="true" aria-label={title}>
      <div className="panel flex h-full max-h-[640px] w-full max-w-5xl flex-col overflow-hidden bg-surface shadow-2xl">
        <div className="flex h-14 shrink-0 items-center gap-3 border-b border-line px-5">
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-semibold">{title}</div>
            {job?.version && <div className="font-mono text-xs text-muted">{job.version}</div>}
          </div>
          {visibleSteps.length > 1 && (
            <div className="font-mono text-xs text-muted tabular-nums">
              Шаг {pos + 1} из {visibleSteps.length}
            </div>
          )}
          <button onClick={onClose} title="Закрыть: выбор можно продолжить из списка загрузок" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <X size={16} />
          </button>
        </div>

        {loadError && <p className="p-6 text-[13px] text-bad">{loadError}</p>}
        {!loadError && (!wizard || !state) && (
          <div className="flex flex-1 items-center justify-center gap-2 text-[13px] text-muted">
            <Loader2 size={16} className="animate-spin" />
            Чтение установщика…
          </div>
        )}

        {wizard && state && !step && (
          <p className="flex-1 p-6 text-[13px] text-muted">У этого установщика нет вариантов на выбор: ставятся только обязательные файлы.</p>
        )}

        {wizard && state && step && (
          <div className="flex min-h-0 flex-1">
            <div className="min-w-0 flex-1 overflow-y-auto p-5">
              {step.name && <h2 className="mb-4 text-base font-semibold">{step.name}</h2>}
              <div className="space-y-5">
                {step.groups.map((g, gi) => {
                  const radio = g.kind === "exactlyOne" || g.kind === "atMostOne";
                  const empty = (g.kind === "exactlyOne" || g.kind === "atLeastOne") && state.selection[at][gi].length === 0;
                  return (
                    <fieldset key={gi}>
                      <legend className="label mb-2 flex items-center gap-2">
                        {g.name}
                        <span className="font-normal normal-case tracking-normal text-faint">{groupHints[g.kind]}</span>
                        {empty && <span className="font-normal normal-case tracking-normal text-warn">выберите вариант</span>}
                      </legend>
                      <ul className="space-y-1">
                        {g.plugins.map((p, pi) => {
                          const kind = state.kinds[at][gi][pi];
                          const on = state.selection[at][gi].includes(pi);
                          const locked = kind === "required" || kind === "notUsable" || g.kind === "all";
                          const active = focused?.group === gi && focused.plugin === pi;
                          return (
                            <li key={pi}>
                              <button
                                role={radio ? "radio" : "checkbox"}
                                aria-checked={on}
                                aria-disabled={locked}
                                onClick={() => {
                                  setFocus({ group: gi, plugin: pi });
                                  click(gi, pi);
                                }}
                                onMouseEnter={() => setFocus({ group: gi, plugin: pi })}
                                className={`flex w-full items-center gap-3 rounded-xl px-3 py-2 text-left text-sm transition-colors ${
                                  active ? "bg-raised" : "hover:bg-raised/60"
                                } ${kind === "notUsable" ? "text-faint" : ""}`}
                              >
                                <span
                                  className={`inline-flex size-4 shrink-0 items-center justify-center ${radio ? "rounded-full" : "rounded"} border ${
                                    on ? "border-neon bg-neon text-bg" : "border-line"
                                  }`}
                                >
                                  {on && (radio ? <span className="size-1.5 rounded-full bg-bg" /> : <Check size={12} strokeWidth={3} />)}
                                </span>
                                <span className="min-w-0 flex-1 truncate">{p.name}</span>
                                {kindHints[kind] && (
                                  <span className={`text-[11px] ${kind === "recommended" ? "text-ok" : "text-faint"}`}>{kindHints[kind]}</span>
                                )}
                                {locked && kind !== "recommended" && <Lock size={12} className="shrink-0 text-faint" />}
                              </button>
                            </li>
                          );
                        })}
                      </ul>
                    </fieldset>
                  );
                })}
              </div>
            </div>
            <aside className="flex w-80 shrink-0 flex-col overflow-y-auto border-l border-line bg-bg/40 p-5">
              {image && <img src={image} alt="" draggable={false} className="mb-4 max-h-56 w-full rounded-xl object-contain" />}
              {plugin ? (
                <>
                  <div className="text-sm font-medium">{plugin.name}</div>
                  <p className="mt-2 text-[13px] whitespace-pre-line text-muted select-text">{plugin.description || "Без описания"}</p>
                </>
              ) : (
                <p className="text-[13px] text-faint">Наведите на вариант, чтобы увидеть описание.</p>
              )}
            </aside>
          </div>
        )}

        {wizard && state && (
          <div className="flex h-16 shrink-0 items-center gap-2 border-t border-line px-5">
            {wizard.previous && (
              <Button
                variant="ghost"
                disabled={busy || !state.valid.every(Boolean)}
                title="Те же варианты, что у установленной версии"
                onClick={() => install(shown(state))}
              >
                <History size={15} />
                Как в прошлый раз
              </Button>
            )}
            <div className="flex-1" />
            {pos > 0 && (
              <Button variant="ghost" onClick={() => go(visibleSteps[pos - 1])} disabled={busy}>
                <ArrowLeft size={15} />
                Назад
              </Button>
            )}
            {last ? (
              <Button variant="primary" disabled={busy || !state.valid.every(Boolean)} onClick={() => install(shown(state))}>
                {busy && <Loader2 size={15} className="animate-spin" />}
                Установить
              </Button>
            ) : (
              <Button variant="primary" disabled={!state.valid[at]} onClick={() => go(visibleSteps[pos + 1])}>
                Далее
                <ArrowRight size={15} />
              </Button>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

function firstPicked(selection: number[][]): { group: number; plugin: number } | null {
  const group = selection.findIndex((g) => g.length > 0);
  return group < 0 ? null : { group, plugin: selection[group][0] };
}
