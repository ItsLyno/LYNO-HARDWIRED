import { ArrowDown, ArrowUp, ChevronRight, RefreshCw, Search, TriangleAlert, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, OVERWRITE, type ModConflicts, type Overlap } from "../api";
import { plural } from "../format";
import { useApp } from "../store";

/**
 * MO2's conflicts tab told from the game's side: for every file several enabled mods ship, whose copy the game gets.
 * A mod hidden by a higher one only "loses", even where it is above a third one: that file never reaches the game.
 */
export function ConflictViewer({ onClose, initial }: { onClose: () => void; initial?: string }) {
  const { build } = useApp();
  const [mods, setMods] = useState<ModConflicts[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [current, setCurrent] = useState<string | null>(initial ?? null);
  const [query, setQuery] = useState("");

  const titles = new Map((build?.mods ?? []).flatMap((m) => (m.kind === "mod" && m.title ? [[m.name, m.title] as const] : [])));
  const title = (folder: string) => (folder === OVERWRITE ? "Overwrite" : (titles.get(folder) ?? folder));

  const refresh = async () => {
    setError(null);
    try {
      const list = await api.modConflicts();
      setMods(list);
      setCurrent((c) => (list.some((m) => m.modFolder === c) ? c : (list[0]?.modFolder ?? null)));
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    void refresh();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, []);

  const q = query.trim().toLowerCase();
  const shown = (mods ?? []).filter((m) => !q || title(m.modFolder).toLowerCase().includes(q) || m.modFolder.toLowerCase().includes(q));
  const mod = mods?.find((m) => m.modFolder === current);

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-8" role="dialog" aria-modal="true" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="panel flex h-full max-h-[960px] w-full max-w-[1500px] flex-col overflow-hidden bg-surface shadow-2xl">
        <div className="flex h-14 shrink-0 items-center gap-3 border-b border-line px-5">
          <div className="min-w-0 flex-1">
            <div className="text-sm font-semibold">Конфликты файлов</div>
            <div className="truncate text-xs text-muted">Один и тот же файл в нескольких включённых модах: игра берёт его из мода выше в списке</div>
          </div>
          <button onClick={() => void refresh()} title="Обновить" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <RefreshCw size={15} />
          </button>
          <button onClick={onClose} aria-label="Закрыть" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <X size={16} />
          </button>
        </div>

        {error ? (
          <p className="px-5 py-8 text-center text-[13px] text-bad">{error}</p>
        ) : mods?.length === 0 ? (
          <p className="px-5 py-8 text-center text-[13px] text-muted">Конфликтов нет: ни один файл не встречается в двух включённых модах.</p>
        ) : (
          <div className="flex min-h-0 flex-1">
            <div className="flex w-80 shrink-0 flex-col border-r border-line">
              <label className="relative m-3 mb-1.5">
                <Search size={15} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-faint" />
                <input
                  value={query}
                  onChange={(e) => setQuery(e.target.value)}
                  placeholder="Поиск мода"
                  spellCheck={false}
                  className="h-8 w-full rounded-full bg-raised pr-4 pl-9 text-[13px] outline-none placeholder:text-faint focus:ring-1 focus:ring-neon/50"
                />
              </label>
              <div className="min-h-0 flex-1 overflow-y-auto py-1.5">
                {mods === null && <p className="px-4 py-2 text-[13px] text-muted">Читаю файлы модов…</p>}
                {shown.map((m) => {
                  const won = m.wins.reduce((n, o) => n + o.files.length, 0);
                  const dead = m.files > 0 && m.lost === m.files;
                  return (
                    <button
                      key={m.modFolder}
                      onClick={() => setCurrent(m.modFolder)}
                      className={`block w-full px-4 py-2 text-left transition-colors ${m.modFolder === current ? "bg-raised" : "hover:bg-raised/50"}`}
                    >
                      <div className={`truncate text-[13px] ${m.modFolder === current ? "text-fg" : "text-muted"}`}>{title(m.modFolder)}</div>
                      <div className="flex items-center gap-3 font-mono text-[11px] tabular-nums">
                        {won > 0 && (
                          <span className="inline-flex items-center gap-0.5 text-ok" title="Его файлы в игре вместо файлов модов ниже">
                            <ArrowUp size={11} />
                            {won}
                          </span>
                        )}
                        {m.lost > 0 && (
                          <span className="inline-flex items-center gap-0.5 text-warn" title="Его файлы закрыты модами выше">
                            <ArrowDown size={11} />
                            {m.lost}
                          </span>
                        )}
                        {dead && <span className="font-sans text-bad">не действует</span>}
                      </div>
                    </button>
                  );
                })}
              </div>
            </div>

            <div className="min-w-0 flex-1 overflow-y-auto px-6 py-5">
              {mod && (
                <>
                  <div className="text-base font-semibold">{title(mod.modFolder)}</div>
                  <div className="mt-0.5 text-xs text-muted">
                    {mod.files} {plural(mod.files, "файл", "файла", "файлов")}, закрыто другими модами: {mod.lost}
                  </div>
                  {mod.files > 0 && mod.lost === mod.files && (
                    <div className="mt-4 flex items-start gap-2 rounded-lg bg-bad/10 px-3 py-2.5 text-[13px] text-bad">
                      <TriangleAlert size={15} className="mt-0.5 shrink-0" />
                      Все файлы мода закрыты модами выше: в игре он ничего не меняет. Поднимите его выше в списке или выключите.
                    </div>
                  )}
                  <Side
                    key={`loses:${mod.modFolder}`}
                    heading="Закрыт модами выше"
                    hint="Эти файлы игра берёт не из этого мода, а из указанного"
                    tone="text-warn"
                    overlaps={mod.loses}
                    title={title}
                    onPick={setCurrent}
                  />
                  <Side
                    key={`wins:${mod.modFolder}`}
                    heading="Закрывает моды ниже"
                    hint="Эти файлы игра берёт из этого мода, а не из указанного"
                    tone="text-ok"
                    overlaps={mod.wins}
                    title={title}
                    onPick={setCurrent}
                  />
                </>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function Side(props: { heading: string; hint: string; tone: string; overlaps: Overlap[]; title: (folder: string) => string; onPick: (folder: string) => void }) {
  const [open, setOpen] = useState<Set<string>>(new Set());
  if (props.overlaps.length === 0) return null;
  const toggle = (folder: string) => {
    const next = new Set(open);
    if (!next.delete(folder)) next.add(folder);
    setOpen(next);
  };
  return (
    <section className="mt-6">
      <h3 className={`text-[13px] font-semibold ${props.tone}`}>{props.heading}</h3>
      <p className="mb-2 text-xs text-faint">{props.hint}</p>
      {props.overlaps.map((o) => (
        <div key={o.modFolder} className="border-b border-line/60">
          <div className="flex h-9 items-center gap-2">
            <button onClick={() => toggle(o.modFolder)} aria-label="Файлы" className="inline-flex size-6 items-center justify-center rounded text-muted hover:bg-raised hover:text-fg">
              <ChevronRight size={14} className={`transition-transform ${open.has(o.modFolder) ? "rotate-90" : ""}`} />
            </button>
            <button onClick={() => props.onPick(o.modFolder)} title="Открыть конфликты этого мода" className="min-w-0 truncate text-[13px] hover:text-neon">
              {props.title(o.modFolder)}
            </button>
            <button onClick={() => toggle(o.modFolder)} className="ml-auto shrink-0 font-mono text-[11px] text-faint tabular-nums hover:text-fg">
              {o.files.length} {plural(o.files.length, "файл", "файла", "файлов")}
            </button>
          </div>
          {open.has(o.modFolder) && (
            <ul className="pb-2 pl-8 font-mono text-[12px] leading-5 text-fg/85 select-text">
              {o.files.map((f) => (
                <li key={f} className="break-all">
                  {f}
                </li>
              ))}
            </ul>
          )}
        </div>
      ))}
    </section>
  );
}
