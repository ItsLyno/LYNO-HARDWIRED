import { Check, Copy, RefreshCw, Search, X } from "lucide-react";
import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { api, type LogFile } from "../api";
import { formatBytes } from "../format";
import { Chip } from "./Chip";

type Level = "error" | "warn";

// RED4ext, TweakXL, ArchiveXL and CET write `[error]` / `[warning]`, redscript `[ERROR - …]` / `[WARN - …]`.
const ERROR = /\b(error|fatal|critical|exception|crash(ed)?|failed)\b/i;
const WARN = /\bwarn(ing)?\b/i;
const levelOf = (line: string): Level | null => (ERROR.test(line) ? "error" : WARN.test(line) ? "warn" : null);

/** Logs the game's frameworks wrote into overwrite/: where to look first when a mod doesn't work or the game crashes. */
export function LogViewer({ onClose }: { onClose: () => void }) {
  const [logs, setLogs] = useState<LogFile[] | null>(null);
  const [path, setPath] = useState<string | null>(null);
  const [text, setText] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState("");
  const [level, setLevel] = useState<Level | null>(null);
  const [copied, setCopied] = useState(false);
  const scroller = useRef<HTMLDivElement>(null);
  // A slow read of a log clicked before must not replace the one clicked last.
  const wanted = useRef<string | null>(null);

  const open = async (p: string | null) => {
    wanted.current = p;
    setPath(p);
    try {
      const t = p ? await api.readOverwriteLog(p) : null;
      if (wanted.current !== p) return;
      setText(t);
      setError(null);
    } catch (e) {
      if (wanted.current !== p) return;
      setText(null);
      setError(String(e));
    }
  };
  const refresh = async () => {
    try {
      const list = await api.overwriteLogs();
      setLogs(list);
      await open(list.find((l) => l.path === wanted.current)?.path ?? list[0]?.path ?? null);
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

  const lines = useMemo(() => (text ?? "").split(/\r?\n/).map((s, i) => ({ n: i + 1, s, level: levelOf(s) })), [text]);
  const errors = lines.filter((l) => l.level === "error").length;
  const warnings = lines.filter((l) => l.level === "warn").length;
  const q = query.trim().toLowerCase();
  const shown = lines.filter((l) => (!level || l.level === level) && (!q || l.s.toLowerCase().includes(q)));

  // The end of a log is the last run: open there.
  useLayoutEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [text, level, q]);

  const copy = async () => {
    await navigator.clipboard.writeText(shown.map((l) => l.s).join("\n"));
    setCopied(true);
    setTimeout(() => setCopied(false), 1500);
  };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-8" role="dialog" aria-modal="true" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="panel flex h-full max-h-[960px] w-full max-w-[1500px] flex-col overflow-hidden bg-surface shadow-2xl">
        <div className="flex h-14 shrink-0 items-center gap-3 border-b border-line px-5">
          <div className="min-w-0 flex-1">
            <div className="text-sm font-semibold">Логи игры</div>
            <div className="truncate text-xs text-muted">Overwrite: что записали RED4ext, CET, redscript, TweakXL и другие моды</div>
          </div>
          <button onClick={() => void refresh()} title="Обновить" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <RefreshCw size={15} />
          </button>
          <button onClick={onClose} aria-label="Закрыть" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <X size={16} />
          </button>
        </div>

        {logs?.length === 0 ? (
          <p className="px-5 py-8 text-center text-[13px] text-muted">Логов пока нет: запустите игру, и моды запишут их сюда.</p>
        ) : (
          <div className="flex min-h-0 flex-1">
            <div className="w-80 shrink-0 overflow-y-auto border-r border-line py-1.5">
              {logs === null && <p className="px-4 py-2 text-[13px] text-muted">Загрузка…</p>}
              {logs?.map((l) => {
                const cut = l.path.lastIndexOf("/");
                return (
                  <button
                    key={l.path}
                    onClick={() => void open(l.path)}
                    title={l.path}
                    className={`block w-full px-4 py-2 text-left transition-colors ${l.path === path ? "bg-raised" : "hover:bg-raised/50"}`}
                  >
                    <div className={`truncate text-[13px] ${l.path === path ? "text-fg" : "text-muted"}`}>{l.path.slice(cut + 1)}</div>
                    <div className="truncate text-[11px] text-faint">{l.path.slice(0, Math.max(cut, 0))}</div>
                    <div className="font-mono text-[11px] text-faint tabular-nums">
                      {new Date(l.modified).toLocaleString("ru-RU", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" })} · {formatBytes(l.size)}
                    </div>
                  </button>
                );
              })}
            </div>

            <div className="flex min-w-0 flex-1 flex-col">
              <div className="flex shrink-0 items-center gap-1.5 border-b border-line px-4 py-2">
                <Chip active={!level} onClick={() => setLevel(null)}>
                  Все
                </Chip>
                <Chip active={level === "error"} onClick={() => setLevel("error")} count={errors}>
                  Ошибки
                </Chip>
                <Chip active={level === "warn"} onClick={() => setLevel("warn")} count={warnings}>
                  Предупреждения
                </Chip>
                <label className="relative ml-auto w-72">
                  <Search size={15} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-faint" />
                  <input
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    placeholder="Поиск в логе"
                    spellCheck={false}
                    className="h-8 w-full rounded-full bg-raised pr-4 pl-9 text-[13px] outline-none placeholder:text-faint focus:ring-1 focus:ring-neon/50"
                  />
                </label>
                <button
                  onClick={() => void copy()}
                  disabled={shown.length === 0}
                  title="Скопировать показанные строки"
                  className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg disabled:text-faint"
                >
                  {copied ? <Check size={15} className="text-ok" /> : <Copy size={15} />}
                </button>
              </div>
              <div ref={scroller} className="min-h-0 flex-1 overflow-auto py-2 font-mono text-[12px] leading-5 select-text">
                {error && <p className="px-4 text-bad">{error}</p>}
                {text !== null && shown.length === 0 && <p className="px-4 text-muted">{q || level ? "Ничего не найдено" : "Лог пуст"}</p>}
                {shown.map((l) => (
                  <div key={l.n} className={`flex gap-3 px-4 whitespace-pre-wrap break-all ${l.level === "error" ? "bg-bad/10 text-bad" : l.level === "warn" ? "text-warn" : "text-fg/85"}`}>
                    <span className="w-12 shrink-0 text-right text-faint select-none tabular-nums">{l.n}</span>
                    <span className="min-w-0">{l.s}</span>
                  </div>
                ))}
              </div>
            </div>
          </div>
        )}
      </div>
    </div>
  );
}
