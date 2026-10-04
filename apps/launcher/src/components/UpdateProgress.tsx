import { api } from "../api";
import { formatBytes } from "../format";
import type { Progress } from "../store";
import { Button } from "./Button";

export function UpdateProgress({ progress }: { progress: Progress }) {
  const { step, bytes } = progress;
  const pct = bytes && bytes.total > 0 ? Math.min(100, (bytes.done / bytes.total) * 100) : null;

  return (
    <div className="rounded-lg border border-line bg-surface px-5 py-4">
      <div className="flex items-baseline gap-3">
        <div className="min-w-0 flex-1 truncate">{step?.label ?? "Подготовка…"}</div>
        {step && (
          <div className="text-[13px] text-muted tabular-nums">
            {step.index} из {step.total}
          </div>
        )}
      </div>
      <div className="mt-3 h-1.5 overflow-hidden rounded-full bg-raised">
        <div
          className="h-full rounded-full bg-accent transition-[width] duration-200"
          style={{ width: `${pct ?? (step ? (step.index / step.total) * 100 : 0)}%` }}
        />
      </div>
      <div className="mt-3 flex items-center justify-between text-[13px] text-muted tabular-nums">
        <span>{bytes ? `${formatBytes(bytes.done)} из ${formatBytes(bytes.total)}` : " "}</span>
        <Button variant="ghost" className="-mr-2 h-7 px-2" onClick={() => api.cancelUpdate()}>
          Отменить
        </Button>
      </div>
    </div>
  );
}
