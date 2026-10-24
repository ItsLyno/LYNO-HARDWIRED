import { FileWarning, ScrollText, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type RunReport } from "../api";
import { plural } from "../format";
import { useApp } from "../store";
import { Button } from "./Button";
import { LogViewer } from "./LogViewer";

/** What the frameworks' logs say went wrong in the last run, grouped by the mod each error points at. */
export function RunProblems({ report, onClose }: { report: RunReport; onClose: () => void }) {
  const { run, dismissRunReport } = useApp();
  const [log, setLog] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  useEffect(() => {
    const esc = (e: KeyboardEvent) => e.key === "Escape" && !log && onClose();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [log]);

  if (log) return <LogViewer initialPath={log} onClose={() => setLog(null)} />;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-8" role="dialog" aria-modal="true" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="panel flex max-h-full w-full max-w-3xl flex-col overflow-hidden bg-surface shadow-2xl">
        <div className="flex h-14 shrink-0 items-center gap-3 border-b border-line px-5">
          <FileWarning size={16} className="text-bad" />
          <div className="min-w-0 flex-1 truncate text-sm font-semibold">Ошибки последнего запуска</div>
          <button onClick={onClose} aria-label="Закрыть" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <X size={16} />
          </button>
        </div>
        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-5 py-4 text-[13px]">
          {report.scriptsFailed ? (
            <p className="text-muted">
              redscript не собрал скрипты: в этом запуске не работал ни один мод на redscript. Обычно виноват мод, на
              который указывает первая ошибка, — выключите его или поставьте другую версию. Если ошибки во многих модах
              сразу, скорее всего игра обновилась, а моды-фреймворки ещё нет.
            </p>
          ) : (
            <p className="text-muted">
              Моды записали в логи ошибки. Не каждая ломает игру; если что-то не работает, начните с мода, на который
              указывает ошибка.
            </p>
          )}
          {report.problems.map((p) => (
            <div key={`${p.log}|${p.modFolder ?? ""}`} className="rounded-lg border border-line/60 bg-raised/30">
              <div className="flex items-center gap-3 px-4 py-2.5">
                <div className="min-w-0 flex-1">
                  <div className="truncate font-medium text-fg">{p.modFolder ?? "Мод не определён"}</div>
                  <div className="truncate text-[12px] text-muted">
                    {p.source} · {p.count} {plural(p.count, "ошибка", "ошибки", "ошибок")}
                  </div>
                </div>
                <Button variant="ghost" className="h-8 text-[13px]" onClick={() => setLog(p.log)} title={p.log}>
                  <ScrollText size={14} />
                  Лог
                </Button>
              </div>
              <pre className="overflow-x-auto border-t border-line/60 px-4 py-2.5 font-mono text-[11.5px] leading-relaxed whitespace-pre-wrap text-fg/80">
                {p.samples.join("\n\n")}
              </pre>
            </div>
          ))}
          {saved && <p className="text-muted">Отчёт сохранён: {saved}</p>}
        </div>
        <div className="flex items-center justify-between gap-2 border-t border-line px-5 py-3">
          <Button
            variant="ghost"
            onClick={() => {
              dismissRunReport(true);
              onClose();
            }}
            title="Эти же ошибки после следующих запусков не покажутся. Новые — покажутся"
          >
            Больше не показывать эти
          </Button>
          <div className="flex gap-2">
            <Button
              onClick={() =>
                run(async () => {
                  setSaved(await api.exportReport());
                })
              }
            >
              Отчёт для автора
            </Button>
            <Button variant="primary" autoFocus onClick={onClose}>
              Закрыть
            </Button>
          </div>
        </div>
      </div>
    </div>
  );
}
