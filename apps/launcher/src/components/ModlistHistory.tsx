import { History, Loader2, Undo2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type HistoryRow } from "../api";
import { plural } from "../format";
import { useApp } from "../store";
import { Button } from "./Button";

/** Earlier states of the mod list, taken before the launcher changed it: one click back after a wrong change. */
export function ModlistHistory({ locked, lockReason, onClose, onRestored }: { locked: boolean; lockReason: string; onClose: () => void; onRestored: () => Promise<void> }) {
  const { run } = useApp();
  const [rows, setRows] = useState<HistoryRow[] | null>(null);
  const [busy, setBusy] = useState<number | null>(null);

  const load = () => run(async () => setRows(await api.modlistHistory()));
  useEffect(() => {
    void load();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, []);

  const restore = (id: number) =>
    run(async () => {
      setBusy(id);
      try {
        await api.modlistRestore(id);
        await onRestored();
        onClose();
      } finally {
        setBusy(null);
      }
    });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-8" role="dialog" aria-modal="true" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="panel flex max-h-full w-full max-w-2xl flex-col overflow-hidden bg-surface shadow-2xl">
        <div className="flex h-14 shrink-0 items-center gap-3 border-b border-line px-5">
          <History size={16} className="text-muted" />
          <div className="min-w-0 flex-1 truncate text-sm font-semibold">История списка модов</div>
          <button onClick={onClose} aria-label="Закрыть" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <X size={16} />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4 text-[13px]">
          <p className="mb-3 text-muted">
            Лаунчер запоминает список перед каждым изменением: включение и выключение, порядок, установка и удаление
            модов, обновление сборки, запуск игры. Возвращаются порядок и включённость, файлы модов — нет: удалённый мод
            не вернётся, установленный позже останется, но выключится.
          </p>
          {rows === null ? (
            <p className="text-muted">Загрузка…</p>
          ) : rows.length === 0 ? (
            <p className="text-muted">Пока ничего: снимки появятся после первых изменений.</p>
          ) : (
            <ul className="divide-y divide-line/60">
              {rows.map((r) => (
                <li key={r.id} className="flex items-center gap-3 py-2.5">
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-fg">
                      {r.label}
                      {r.more > 0 && <span className="text-muted"> и ещё {r.more} {plural(r.more, "изменение", "изменения", "изменений")}</span>}
                    </div>
                    <div className="truncate text-[12px] text-muted">
                      {new Date(r.id).toLocaleString("ru-RU", { day: "numeric", month: "long", hour: "2-digit", minute: "2-digit" })}
                      {" · "}
                      {r.blocked ?? describe(r.diff)}
                    </div>
                  </div>
                  <Button
                    className="h-8 shrink-0 text-[13px]"
                    disabled={!!r.blocked || locked || busy !== null || isSame(r.diff)}
                    title={r.blocked ?? (locked ? lockReason : isSame(r.diff) ? "Список сейчас такой же" : undefined)}
                    onClick={() => restore(r.id)}
                  >
                    {busy === r.id ? <Loader2 size={14} className="animate-spin" /> : <Undo2 size={14} />}
                    Вернуть
                  </Button>
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

const isSame = (d: HistoryRow["diff"]) => d.switched === 0 && d.off === 0 && !d.moved;

function describe(d: HistoryRow["diff"]): string {
  const parts: string[] = [];
  if (d.switched > 0) parts.push(`${d.switched} ${plural(d.switched, "мод переключится", "мода переключатся", "модов переключатся")}`);
  if (d.off > 0) parts.push(`${d.off} ${plural(d.off, "новый выключится", "новых выключатся", "новых выключатся")}`);
  if (d.moved) parts.push("порядок изменится");
  if (d.gone > 0) parts.push(`${d.gone} ${plural(d.gone, "удалённый не вернётся", "удалённых не вернутся", "удалённых не вернутся")}`);
  return parts.length > 0 ? parts.join(", ") : "как сейчас";
}
