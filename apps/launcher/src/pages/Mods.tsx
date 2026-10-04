import { ChevronRight, ExternalLink, FolderOpen, Lock, Search } from "lucide-react";
import { useEffect, useState, type CSSProperties } from "react";
import { api, type ModRow, type UserRow } from "../api";
import { PageTitle } from "../components/PageTitle";
import { formatBytes, plural } from "../format";
import { useApp } from "../store";

type Mod = Extract<ModRow, { kind: "mod" }>;
type Group = { title: string | null; color: string | null; mods: Mod[] };
type Filter = "all" | "off" | "changes";

/** MO2's folder of the separator above the player's own mods (`plan::USER_SEPARATOR`). */
const USER_SEPARATOR = "LYNO USER MODS_separator";

export function Mods() {
  const { build, buildError, status, progress, run, setModEnabled, settings, nexusUpdates, userMods, refreshUserMods, drag, fileOver } =
    useApp();
  useEffect(() => {
    void refreshUserMods();
  }, []);
  // The author takes Nexus updates of build mods into the next release (tab «Nexus»).
  const nexusUpdate = new Map(
    settings?.authorMode
      ? (nexusUpdates?.mods ?? []).flatMap((m) => (m.status.kind === "update" ? [[m.folder, m.status.version] as const] : []))
      : [],
  );
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [collapsed, setCollapsed] = useState<Set<string>>(loadCollapsed);
  const toggleGroup = (title: string) => {
    const next = new Set(collapsed);
    if (!next.delete(title)) next.add(title);
    setCollapsed(next);
    saveCollapsed(next);
  };

  const isChange = (m: Mod) => !!m.recent || m.outdated || m.damaged || (!m.installed && !!build?.installedVersion);
  const keep = (m: Mod) => filter === "all" || (filter === "off" ? !m.enabled : isChange(m));
  const groups = groupMods(build?.mods ?? [], query, keep);
  const all = (build?.mods ?? []).filter((m): m is Mod => m.kind === "mod");
  const shown = groups.reduce((n, g) => n + g.mods.length, 0);
  const totalSize = all.reduce((n, m) => n + m.size, 0);
  const offCount = all.filter((m) => !m.enabled).length;
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
  // Where an archive being dragged or dropped from Explorer would land. A player's mod never goes among the
  // build's: over those it lands at the top of their own section, which is what the core does with it too.
  const spot = drag?.over ?? (fileOver && !fileOver.outside ? fileOver : null);
  const dropping = !!drag || (!!fileOver && !fileOver.outside);
  const buildNames = new Set(build.mods.map((r) => (r.kind === "mod" ? r.name : `${r.title}_separator`)));
  const lineAfter = spot && (spot.after === null ? "end" : !settings?.authorMode && buildNames.has(spot.after) ? USER_SEPARATOR : spot.after);
  const userRows = (userMods ?? []).filter((r) => r.kind === "separator" || filter === "all" || (filter === "off" && !r.enabled));
  const q = query.trim().toLowerCase();
  const shownUser = userRows.filter((r) => !q || r.kind === "separator" || r.name.toLowerCase().includes(q));
  const endKey = shownUser.length > 0 ? rowKey(shownUser[shownUser.length - 1]) : USER_SEPARATOR;
  const line = (key: string) => (lineAfter === key || (lineAfter === "end" && key === endKey) ? "shadow-[inset_0_-2px_0_0_var(--color-neon)]" : "");

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
            className="h-9 w-full rounded-full bg-surface pr-4 pl-9 text-sm outline-none placeholder:text-faint focus:ring-1 focus:ring-neon/50"
          />
        </label>
      </div>

      {(offCount > 0 || changeCount > 0) && (
        <div className="mt-4 flex items-center gap-1.5">
          <Chip active={filter === "all"} onClick={() => setFilter("all")}>
            Все
          </Chip>
          {offCount > 0 && (
            <Chip active={filter === "off"} onClick={() => setFilter("off")} count={offCount}>
              Выключенные
            </Chip>
          )}
          {changeCount > 0 && (
            <Chip active={filter === "changes"} onClick={() => setFilter("changes")} count={changeCount}>
              Изменения
            </Chip>
          )}
        </div>
      )}
      {filter === "off" && (
        <p className="mt-3 text-[13px] text-muted">
          Выбор сохраняется при обновлениях сборки. Выключенный мод остаётся установленным. Если от мода зависят
          другие, выключите и их: лаунчер зависимостей не проверяет.
        </p>
      )}
      {filter === "changes" && last && last.removed.length > 0 && (
        <p className="mt-3 text-[13px] text-muted">
          Удалены в {last.to}: {last.removed.join(", ")}
        </p>
      )}

      <div data-drop-zone className={`panel mt-4 min-h-0 flex-1 overflow-y-auto transition-shadow ${dropping ? "ring-1 ring-neon/40" : ""}`}>
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
                <SeparatorRow
                  dropKey={`${g.title}_separator`}
                  lineClass={line(`${g.title}_separator`)}
                  title={g.title}
                  color={g.color}
                  count={g.mods.length}
                  // A search shows every match, folded or not.
                  open={!!query.trim() || !collapsed.has(g.title)}
                  onToggle={() => toggleGroup(g.title!)}
                />
              )}
              {(!g.title || query.trim() || !collapsed.has(g.title)) && g.mods.map((m) => (
                <tr key={m.id} data-drop-after={m.name} className={`h-12 border-b border-line/60 last:border-b-0 hover:bg-raised/50 ${line(m.name)}`}>
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
                    {nexusUpdate.has(m.name) && (
                      <Badge tone="ok" title={`На Nexus: ${nexusUpdate.get(m.name) ?? "новая версия"}. Обновить — на вкладке «Nexus»`}>
                        обновление на Nexus
                      </Badge>
                    )}
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
                      {m.installed && !m.optional && (
                        <span
                          className="mr-1 inline-flex h-[18px] w-8 items-center justify-center text-faint"
                          title="Основа сборки: от этого мода зависят другие, выключить его нельзя"
                        >
                          <Lock size={13} />
                        </span>
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
          {(shownUser.length > 0 || dropping) && filter !== "changes" && (
            <tbody>
              <SeparatorRow
                title="Мои моды"
                color={null}
                count={shownUser.filter((r) => r.kind === "mod").length}
                open={!!q || dropping || !collapsed.has(USER_SEPARATOR)}
                onToggle={() => toggleGroup(USER_SEPARATOR)}
                dropKey={USER_SEPARATOR}
                lineClass={line(USER_SEPARATOR)}
              />
              {(q || dropping || !collapsed.has(USER_SEPARATOR)) &&
                shownUser.map((r) => <UserModRow key={rowKey(r)} row={r} locked={locked} lockReason={lockReason} lineClass={line(rowKey(r))} />)}
              {dropping && shownUser.length === 0 && (
                <tr data-drop-after={USER_SEPARATOR}>
                  <td colSpan={5} className="px-5 py-4 text-center text-[13px] text-neon">
                    Отпустите архив, чтобы установить его в ваши моды
                  </td>
                </tr>
              )}
            </tbody>
          )}
        </table>
        {shown === 0 && shownUser.length === 0 && !dropping && <p className="px-5 py-8 text-center text-[13px] text-muted">Ничего не найдено</p>}
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
      className={`h-8 rounded-full px-3.5 text-[13px] transition-colors ${
        props.active ? "bg-raised text-fg" : "text-muted hover:bg-raised/60 hover:text-fg"
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
        props.checked ? "bg-ok/80" : "bg-line"
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

function rowKey(r: UserRow): string {
  return r.kind === "mod" ? r.name : `${r.title}_separator`;
}

// The player's own mods: theirs to switch, in MO2's list order. Updates of the build never touch them.
function UserModRow(props: { row: UserRow; locked: boolean; lockReason: string; lineClass: string }) {
  const { run, refreshUserMods } = useApp();
  const r = props.row;
  if (r.kind === "separator") {
    return (
      <tr data-drop-after={rowKey(r)} className={`h-9 border-b border-line/60 ${props.lineClass}`}>
        <td colSpan={5} className="pl-5 text-[12px] font-semibold text-muted">
          {r.title}
        </td>
      </tr>
    );
  }
  const toggle = (enabled: boolean) =>
    run(async () => {
      await api.setUserModEnabled(r.name, enabled);
      await refreshUserMods();
    });
  return (
    <tr data-drop-after={r.name} className={`h-12 border-b border-line/60 last:border-b-0 hover:bg-raised/50 ${props.lineClass}`}>
      <td className="truncate pl-5">
        <span className={r.enabled ? "" : "text-faint"}>{r.name}</span>
        {!r.enabled && <Badge>выключен</Badge>}
      </td>
      <td className="truncate text-muted">—</td>
      <td className="truncate font-mono text-xs text-muted tabular-nums">{r.version ?? "—"}</td>
      <td className="pr-4 text-right font-mono text-xs text-muted tabular-nums">—</td>
      <td className="pr-5">
        <div className="flex items-center justify-end gap-1">
          <Switch
            checked={r.enabled}
            disabled={props.locked}
            title={props.locked ? props.lockReason : r.enabled ? "Выключить мод" : "Включить мод"}
            onChange={toggle}
          />
          <button
            onClick={() => run(() => api.openUserModFolder(r.name))}
            className="inline-flex h-7 w-7 items-center justify-center rounded-full text-muted transition-colors hover:bg-raised hover:text-fg"
            title="Открыть папку мода"
          >
            <FolderOpen size={14} />
          </button>
          {r.nexusUrl && (
            <button
              onClick={() => run(() => api.openUrl(r.nexusUrl!))}
              className="inline-flex h-7 items-center gap-1.5 rounded-full px-2.5 text-[13px] text-muted transition-colors hover:bg-raised hover:text-fg"
              title={r.nexusUrl}
            >
              Nexus
              <ExternalLink size={13} />
            </button>
          )}
        </div>
      </td>
    </tr>
  );
}

// Separators come from the author's MO2 (title, order and color); like in MO2, a click folds the group.
function SeparatorRow(props: {
  title: string;
  color: string | null;
  count: number;
  open: boolean;
  onToggle: () => void;
  dropKey?: string;
  lineClass?: string;
}) {
  // Opaque background: the row sticks under the table header while its group scrolls by.
  const style: CSSProperties = props.color
    ? {
        background: `linear-gradient(color-mix(in srgb, ${props.color} 14%, transparent), color-mix(in srgb, ${props.color} 14%, transparent)), var(--color-surface)`,
        color: `color-mix(in srgb, ${props.color} 55%, var(--color-fg))`,
      }
    : { background: "var(--color-raised)" };
  return (
    <tr data-drop-after={props.dropKey} className={props.lineClass}>
      <td colSpan={5} className="sticky top-10 z-[5] p-0">
        <button
          onClick={props.onToggle}
          aria-expanded={props.open}
          style={style}
          className="flex h-10 w-full items-center gap-2 pr-5 pl-4 text-left text-[13px] font-semibold transition-[filter] hover:brightness-125"
        >
          <ChevronRight size={14} className={`shrink-0 opacity-70 transition-transform ${props.open ? "rotate-90" : ""}`} />
          <span className="truncate">{props.title}</span>
          <span className="font-mono text-[11px] font-normal opacity-60 tabular-nums">{props.count}</span>
        </button>
      </td>
    </tr>
  );
}

const COLLAPSED_KEY = "mods.collapsed";

function loadCollapsed(): Set<string> {
  try {
    return new Set(JSON.parse(localStorage.getItem(COLLAPSED_KEY) ?? "[]") as string[]);
  } catch {
    return new Set();
  }
}

function saveCollapsed(titles: Set<string>) {
  try {
    localStorage.setItem(COLLAPSED_KEY, JSON.stringify([...titles]));
  } catch {
    // Folding is a convenience; losing it is fine.
  }
}

function groupMods(rows: ModRow[], query: string, keep: (m: Mod) => boolean): Group[] {
  const q = query.trim().toLowerCase();
  const matches = (m: Mod) =>
    keep(m) && (!q || [m.name, m.title, m.author].some((s) => s?.toLowerCase().includes(q)));
  const groups: Group[] = [{ title: null, color: null, mods: [] }];
  for (const r of rows) {
    if (r.kind === "separator") groups.push({ title: r.title, color: r.color, mods: [] });
    else if (matches(r)) groups[groups.length - 1].mods.push(r);
  }
  return groups.filter((g) => g.mods.length > 0);
}
