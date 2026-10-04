import { RefreshCw, X } from "lucide-react";
import { useEffect } from "react";
import { useApp } from "../store";
import { Button } from "./Button";

/**
 * An archive dragged or dropped onto the list is another version of an
 * installed mod: it goes over that mod's folder, so the player says yes first.
 */
export function ReplaceConfirm() {
  const ask = useApp((s) => s.replaceAsks[0]);
  const answerReplace = useApp((s) => s.answerReplace);

  useEffect(() => {
    if (!ask) return;
    const esc = (e: KeyboardEvent) => e.key === "Escape" && answerReplace(false);
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, [ask]);

  if (!ask) return null;
  const { target } = ask;
  const from = target.installedVersion ?? "неизвестна";
  const to = target.version ?? "неизвестна";
  const same = target.installedVersion !== null && target.installedVersion === target.version;
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-8" role="dialog" aria-modal="true">
      <div className="panel flex w-full max-w-md flex-col overflow-hidden bg-surface shadow-2xl">
        <div className="flex h-14 shrink-0 items-center gap-3 border-b border-line px-5">
          <RefreshCw size={16} className="text-muted" />
          <div className="min-w-0 flex-1 truncate text-sm font-semibold">{target.replaces}</div>
          <button onClick={() => answerReplace(false)} aria-label="Отмена" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <X size={16} />
          </button>
        </div>
        <div className="space-y-2 px-5 py-4 text-[13px] text-muted">
          <p>
            {same ? "Эта версия уже установлена. Переустановить мод из архива?" : "Мод уже установлен. Заменить его версией из архива?"}
          </p>
          <p className="font-mono text-xs">
            <span>{from}</span>
            <span className="px-2 text-faint">→</span>
            <span className="text-accent">{to}</span>
          </p>
          <p>Файлы мода заменятся целиком, место в списке и включённость сохранятся.</p>
        </div>
        <div className="flex justify-end gap-2 border-t border-line px-5 py-3">
          <Button variant="ghost" onClick={() => answerReplace(false)}>
            Отмена
          </Button>
          <Button variant="primary" autoFocus onClick={() => answerReplace(true)}>
            {same ? "Переустановить" : "Заменить"}
          </Button>
        </div>
      </div>
    </div>
  );
}
