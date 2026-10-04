import { ExternalLink, FolderOpen, Search } from "lucide-react";
import { useState } from "react";
import { api, type ModRow } from "../api";
import { PageTitle } from "../components/PageTitle";
import { formatBytes, plural } from "../format";
import { useApp } from "../store";

type Mod = Extract<ModRow, { kind: "mod" }>;
type Group = { title: string | null; mods: Mod[] };
type Filter = "all" | "optional" | "changes";

export function Mods() {
  const { build, buildError, status, progress, run, setModEnabled } = useApp();
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");

  const isChange = (m: Mod) => !!m.recent || m.outdated || m.damaged || (!m.installed && !!build?.installedVersion);
  const keep = (m: Mod) => filter === "all" || (filter === "optional" ? m.optional : isChange(m));
  const groups = groupMods(build?.mods ?? [], query, keep);
  const all = (build?.mods ?? []).filter((m): m is Mod => m.kind === "mod");
  const shown = groups.reduce((n, g) => n + g.mods.length, 0);
  const totalSize = all.reduce((n, m) => n + m.size, 0);
  const optionalCount = all.filter((m) => m.optional).length;
  const changeCount = all.filter(isChange).length;
  // MO2 rewrites modlist.txt on exit, so switching mods while it runs would be lost.
  const locked = !!progress || !!status?.gameRunning || !!status?.mo2Running;
  const lockReason = progress
    ? "Дождитесь окончания обновления"
    : "Закройте игру и Mod Organizer 2, чтобы включать и выключать моды";

  if (!build) {
    return <p className="text-muted">{buildError ?? "Загрузка списка модов…"}</p>;
  }
  const last = build.lastUpdate;

  return (
    <div className="mx-auto flex h-full max-w-5xl flex-col">
      <div className="flex items-end justify-between gap-6">
        <PageTitle
          sub={`${all.length} ${plural(all.length, "мод", "мода", "модов")} · ${formatBytes(totalSize)} · сборка ${build.latestVersion}`}
        >
          Моды
        </PageTitle>
        <label className="relative w-72">
          <Search size={15} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-faint" />
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Поиск по названию или автору"
            spellCheck={false}
            className="h-9 w-full rounded-full border border-line bg-surface/80 pr-4 pl-9 text-sm outline-none placeholder:text-faint focus:border-neon/50"
          />
        </label>
      </div>

      {(optionalCount > 0 || changeCount > 0) && (
        <div className="mt-4 flex items-center gap-1.5">
          <Chip active={filter === "all"} onClick={() => setFilter("all")}>
            Все
          </Chip>
          {optionalCount > 0 && (
            <Chip active={filter === "optional"} onClick={() => setFilter("optional")} count={optionalCount}>
              Опциональные
            </Chip>
          )}
          {changeCount > 0 && (
            <Chip active={filter === "changes"} onClick={() => setFilter("changes")} count={changeCount}>
              Изменения
            </Chip>
          )}
        </div>
      )}
      {filter === "optional" && (
        <p className="mt-3 text-[13px] text-muted">
          Эти моды можно включать и выключать: выбор сохранится при обновлениях сборки. Выключенный мод остаётся
          установленным.
        </p>
      )}
      {filter === "changes" && last && last.removed.length > 0 && (
        <p className="mt-3 text-[13px] text-muted">
          Удалены в {last.to}: {last.removed.join(", ")}
        </p>
      )}

      <div className="panel mt-4 min-h-0 flex-1 overflow-y-auto">
        <table className="w-full table-fixed text-left">
          <colgroup>
            <col />
            <col className="w-48" />
            <col className="w-28" />
            <col className="w-24" />
            <col className="w-40" />
          </colgroup>
          <thead className="sticky top-0 z-10 bg-surface">
            <tr className="label h-10 border-b border-line">
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
                <tr className="h-10 border-b border-line/60">
                  <td colSpan={5} className="pl-5">
                    <span className="label text-fg/80">{g.title}</span>
                    <span className="ml-2 font-mono text-[11px] text-faint tabular-nums">{g.mods.length}</span>
                  </td>
                </tr>
              )}
              {g.mods.map((m) => (
                <tr key={m.id} className="h-12 border-b border-line/60 last:border-b-0 hover:bg-raised/50">
                  <td className="truncate pl-5">
                    <span className={m.enabled ? "" : "text-faint"}>{m.title ?? m.name}</span>
                    {!m.enabled && <Badge>выключен</Badge>}
                    {m.recent && last && (
                      <Badge tone="ok" title={`В версии ${last.to}`}>
                        {m.recent === "added" ? "добавлен" : "обновлён"}
                      </Badge>
                    )}
                    {m.outdated && <Badge tone="warn">обновится</Badge>}
                    {m.damaged && !m.outdated && (
                      <Badge tone="warn" title="Проверка нашла изменённые или удалённые файлы">
                        восстановится
                      </Badge>
                    )}
                    {!m.installed && build.installedVersion && <Badge tone="warn">новый</Badge>}
                  </td>
                  <td className="truncate text-muted">{m.author ?? "—"}</td>
                  <td className="truncate font-mono text-xs text-muted tabular-nums">{m.version ?? "—"}</td>
                  <td className="pr-4 text-right font-mono text-xs text-muted tabular-nums">{formatBytes(m.size)}</td>
                  <td className="pr-5">
                    <div className="flex items-center justify-end gap-1">
                      {m.optional && m.installed && (
                        <Switch
                          checked={m.enabled}
                          disabled={locked}
                          title={locked ? lockReason : m.enabled ? "Выключить мод" : "Включить мод"}
                          onChange={(v) => setModEnabled(m.id, v)}
                        />
                      )}
                      {m.installed && (
                        <button
                          onClick={() => run(() => api.openModFolder(m.id))}
                          className="inline-flex h-7 w-7 items-center justify-center rounded-full text-muted transition-colors hover:bg-raised hover:text-fg"
                          title="Открыть папку мода"
                        >
                          <FolderOpen size={14} />
                        </button>
                      )}
                      {m.nexusUrl && (
                        <button
                          onClick={() => run(() => api.openUrl(m.nexusUrl!))}
                          className="inline-flex h-7 items-center gap-1.5 rounded-full px-2.5 text-[13px] text-muted transition-colors hover:bg-raised hover:text-fg"
                          title={m.nexusUrl}
                        >
                          Nexus
                          <ExternalLink size={13} />
                        </button>
                      )}
                    </div>
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

const badgeTones = {
  muted: "bg-raised text-muted",
  warn: "bg-warn/10 text-warn",
  ok: "bg-ok/10 text-ok",
};

function Badge({ children, tone = "muted", title }: { children: string; tone?: keyof typeof badgeTones; title?: string }) {
  return (
    <span title={title} className={`ml-2 rounded-full px-2 py-0.5 align-[1px] text-[11px] ${badgeTones[tone]}`}>
      {children}
    </span>
  );
}

function Chip(props: { active: boolean; onClick: () => void; count?: number; children: string }) {
  return (
    <button
      onClick={props.onClick}
      className={`h-8 rounded-full border px-3.5 text-[13px] transition-colors ${
        props.active ? "border-neon/30 bg-raised text-fg" : "border-transparent text-muted hover:bg-raised/60 hover:text-fg"
      }`}
    >
      {props.children}
      {props.count !== undefined && <span className="ml-1.5 font-mono text-[11px] text-faint tabular-nums">{props.count}</span>}
    </button>
  );
}

function Switch(props: { checked: boolean; disabled?: boolean; title: string; onChange: (v: boolean) => void }) {
  return (
    <button
      role="switch"
      aria-checked={props.checked}
      disabled={props.disabled}
      title={props.title}
      onClick={() => props.onChange(!props.checked)}
      className={`relative mr-1 inline-flex h-[18px] w-8 shrink-0 items-center rounded-full transition-colors disabled:cursor-not-allowed disabled:opacity-50 ${
        props.checked ? "bg-ok/80 shadow-[0_0_10px_-2px_var(--color-ok)]" : "bg-line"
      }`}
    >
      <span
        className={`inline-block h-3.5 w-3.5 rounded-full bg-fg transition-transform ${
          props.checked ? "translate-x-4" : "translate-x-0.5"
        }`}
      />
    </button>
  );
}

function groupMods(rows: ModRow[], query: string, keep: (m: Mod) => boolean): Group[] {
  const q = query.trim().toLowerCase();
  const matches = (m: Mod) =>
    keep(m) && (!q || [m.name, m.title, m.author].some((s) => s?.toLowerCase().includes(q)));
  const groups: Group[] = [{ title: null, mods: [] }];
  for (const r of rows) {
    if (r.kind === "separator") groups.push({ title: r.title, mods: [] });
    else if (matches(r)) groups[groups.length - 1].mods.push(r);
  }
  return groups.filter((g) => g.mods.length > 0);
}
