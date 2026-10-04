import { ExternalLink, Search } from "lucide-react";
import { useMemo, useState } from "react";
import { api, type ModRow } from "../api";
import { formatBytes, plural } from "../format";
import { useApp } from "../store";

type Mod = Extract<ModRow, { kind: "mod" }>;
type Group = { title: string | null; mods: Mod[] };

export function Mods() {
  const { build, buildError, run } = useApp();
  const [query, setQuery] = useState("");

  const groups = useMemo(() => groupMods(build?.mods ?? [], query), [build, query]);
  const all = (build?.mods ?? []).filter((m): m is Mod => m.kind === "mod");
  const shown = groups.reduce((n, g) => n + g.mods.length, 0);
  const totalSize = all.reduce((n, m) => n + m.size, 0);

  if (!build) {
    return <p className="text-muted">{buildError ?? "Загрузка списка модов…"}</p>;
  }

  return (
    <div className="mx-auto flex h-full max-w-5xl flex-col">
      <div className="flex items-end justify-between gap-6">
        <div>
          <h1 className="text-2xl font-semibold tracking-tight">Моды</h1>
          <p className="mt-1 text-[13px] text-muted tabular-nums">
            {all.length} {plural(all.length, "мод", "мода", "модов")} · {formatBytes(totalSize)} · сборка {build.latestVersion}
          </p>
        </div>
        <label className="relative w-72">
          <Search size={15} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-faint" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Поиск по названию или автору"
            spellCheck={false}
            className="h-9 w-full rounded-md border border-line bg-surface pr-3 pl-9 text-sm outline-none placeholder:text-faint focus:border-muted"
          />
        </label>
      </div>

      <div className="mt-5 min-h-0 flex-1 overflow-y-auto rounded-lg border border-line bg-surface">
        <table className="w-full table-fixed text-left">
          <colgroup>
            <col />
            <col className="w-48" />
            <col className="w-28" />
            <col className="w-24" />
            <col className="w-28" />
          </colgroup>
          <thead className="sticky top-0 z-10 bg-surface text-xs text-muted">
            <tr className="h-10 border-b border-line">
              <th className="pl-5 font-medium">Название</th>
              <th className="font-medium">Автор</th>
              <th className="font-medium">Версия</th>
              <th className="pr-4 text-right font-medium">Размер</th>
              <th className="pr-5" />
            </tr>
          </thead>
          {groups.map((g, i) => (
            <tbody key={g.title ?? `group-${i}`}>
              {g.title && (
                <tr className="h-9 border-b border-line bg-bg/60">
                  <td colSpan={5} className="pl-5 text-xs font-semibold text-muted">
                    {g.title}
                    <span className="ml-2 font-normal text-faint tabular-nums">{g.mods.length}</span>
                  </td>
                </tr>
              )}
              {g.mods.map((m) => (
                <tr key={m.id} className="h-12 border-b border-line last:border-b-0 hover:bg-raised/50">
                  <td className="truncate pl-5">
                    <span className={m.enabled ? "" : "text-faint"}>{m.title ?? m.name}</span>
                    {!m.enabled && <Badge>выключен</Badge>}
                    {m.outdated && <Badge tone="warn">обновится</Badge>}
                    {!m.installed && build.installedVersion && <Badge tone="warn">новый</Badge>}
                  </td>
                  <td className="truncate text-muted">{m.author ?? "—"}</td>
                  <td className="truncate text-[13px] text-muted tabular-nums">{m.version ?? "—"}</td>
                  <td className="pr-4 text-right text-[13px] text-muted tabular-nums">{formatBytes(m.size)}</td>
                  <td className="pr-5 text-right">
                    {m.nexusUrl && (
                      <button
                        onClick={() => run(() => api.openUrl(m.nexusUrl!))}
                        className="inline-flex h-7 items-center gap-1.5 rounded-md px-2 text-[13px] text-muted transition-colors hover:bg-raised hover:text-fg"
                        title={m.nexusUrl}
                      >
                        Nexus
                        <ExternalLink size={13} />
                      </button>
                    )}
                  </td>
                </tr>
              ))}
            </tbody>
          ))}
        </table>
        {shown === 0 && <p className="px-5 py-8 text-center text-[13px] text-muted">Ничего не найдено</p>}
      </div>

      <p className="mt-4 text-xs text-faint">
        Все моды принадлежат их авторам. Если мод понравился — поддержите автора на Nexus Mods.
      </p>
    </div>
  );
}

function Badge({ children, tone }: { children: string; tone?: "warn" }) {
  return (
    <span
      className={`ml-2 rounded px-1.5 py-0.5 align-[1px] text-[11px] ${
        tone === "warn" ? "bg-warn/10 text-warn" : "bg-raised text-muted"
      }`}
    >
      {children}
    </span>
  );
}

function groupMods(rows: ModRow[], query: string): Group[] {
  const q = query.trim().toLowerCase();
  const matches = (m: Mod) =>
    !q || [m.name, m.title, m.author].some((s) => s?.toLowerCase().includes(q));
  const groups: Group[] = [{ title: null, mods: [] }];
  for (const r of rows) {
    if (r.kind === "separator") groups.push({ title: r.title, mods: [] });
    else if (matches(r)) groups[groups.length - 1].mods.push(r);
  }
  return groups.filter((g) => g.mods.length > 0);
}
