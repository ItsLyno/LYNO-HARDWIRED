import type { ChangelogEntry } from "../api";

export function Changelog({ entries, installed }: { entries: ChangelogEntry[]; installed: string | null }) {
  return (
    <ol className="space-y-5 px-5 pb-5">
      {entries.map((e) => (
        <li key={e.version}>
          <div className="flex items-baseline gap-2">
            <span className="font-mono text-[13px] font-semibold tabular-nums">{e.version}</span>
            {e.version === installed && (
              <span className="rounded-full bg-ok/10 px-2 py-0.5 text-[11px] text-ok">установлена</span>
            )}
            {e.date && (
              <span className="ml-auto font-mono text-[11px] text-faint tabular-nums">
                {new Date(e.date).toLocaleDateString("ru-RU", { day: "numeric", month: "long" })}
              </span>
            )}
          </div>
          <ul className="mt-2 space-y-1 text-[13px] leading-relaxed text-muted">
            {e.notes.map((n) => (
              <li key={n}>{n}</li>
            ))}
          </ul>
        </li>
      ))}
    </ol>
  );
}
