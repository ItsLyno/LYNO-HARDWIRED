import type { ChangelogEntry } from "../api";

export function Changelog({ entries, installed }: { entries: ChangelogEntry[]; installed: string | null }) {
  return (
    <ol className="divide-y divide-line">
      {entries.map((e) => (
        <li key={e.version} className="px-5 py-4">
          <div className="flex items-baseline gap-2">
            <span className="font-semibold tabular-nums">{e.version}</span>
            {e.version === installed && (
              <span className="rounded bg-raised px-1.5 py-0.5 text-[11px] text-muted">установлена</span>
            )}
            {e.date && (
              <span className="ml-auto text-xs text-faint tabular-nums">
                {new Date(e.date).toLocaleDateString("ru-RU", { day: "numeric", month: "long" })}
              </span>
            )}
          </div>
          <ul className="mt-2 space-y-1 text-[13px] leading-relaxed text-muted">
            {e.notes.map((n) => (
              <li key={n} className="flex gap-2">
                <span className="text-faint">–</span>
                {n}
              </li>
            ))}
          </ul>
        </li>
      ))}
    </ol>
  );
}
