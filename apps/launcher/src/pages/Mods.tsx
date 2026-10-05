import { ChevronRight, ExternalLink, Power, PowerOff, FolderOpen, Lock, MoreHorizontal, Pencil, RefreshCw, Search, Trash2, X, type LucideIcon, TriangleAlert, Undo2, ArrowUp, Plus, Wrench, Palette, Eraser, PackagePlus, SeparatorHorizontal, ScrollText } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type MouseEvent, type ReactNode } from "react";
import { api, type ModRow, type Need, type UserRow } from "../api";
import { Button } from "../components/Button";
import { Chip } from "../components/Chip";
import { LogViewer } from "../components/LogViewer";
import { movesSeparator, pressToDrag } from "../components/DragGhost";
import { Notices } from "../components/Notices";
import { PageTitle } from "../components/PageTitle";
import { formatBytes, plural } from "../format";
import { activeInstance, useApp } from "../store";

type Mod = Extract<ModRow, { kind: "mod" }>;
type UserMod = Extract<UserRow, { kind: "mod" }>;
/** A build group shows the player's mods placed among the build's too. */
type Group = { title: string | null; color: string | null; mods: (Mod | UserMod)[] };
type UserSeparator = Extract<UserRow, { kind: "separator" }>;
/** The player's mods under one of their separators; `sep` null: right under the section's header. */
type UserGroup = { sep: UserSeparator | null; mods: UserMod[] };
type Filter = "all" | "off" | "changes";
/** A row the context menu acts on: a mod, a separator of the player or MO2's overwrite. `id`: manifest id of a build mod. */
type Target = {
  kind: "mod" | "separator" | "overwrite";
  folder: string;
  label: string;
  id: string | null;
  nexusUrl: string | null;
  installed: boolean;
  enabled: boolean;
  /** A build mod the player may remove (not core), and one they removed. */
  optional: boolean;
  removed: boolean;
  /** The player's version of this optional build mod. */
  buildId: string | null;
  needs: Need[];
  color: string | null;
};
type Menu = { target: Target; x: number; y: number };
/** `after`: where a new separator goes. */
/** `many`: every selected row a delete goes to. */
type Dialog = { kind: "delete" | "reinstall" | "rename" | "color" | "separator" | "toMod" | "clear" | "logs"; target: Target; after?: string | null; many?: Target[] };

/** MO2's folder of the separator above the player's own mods (`plan::USER_SEPARATOR`). */
const USER_SEPARATOR = "LYNO USER MODS_separator";

export function Mods() {
  const { run, build, buildError, status, progress, setModEnabled, settings, nexus, nexusUpdates, nexusChecking, checkNexus, userMods, refreshUserMods, refreshBuild, drag, fileOver, overwrite } =
    useApp();
  // Both sections mirror MO2, where mods may have changed since the launcher last looked.
  useEffect(() => {
    void refreshUserMods();
    void refreshBuild();
  }, []);
  // A player's build mods come with the build: only the author (`canUpdate`) is shown their Nexus updates.
  const nexusUpdate = new Map(
    (nexusUpdates?.mods ?? []).flatMap((m) => (m.status.kind === "update" && m.canUpdate ? [[m.folder, { version: m.status.version, url: m.pageUrl }] as const] : [])),
  );
  const needs = new Map((nexusUpdates?.mods ?? []).map((m) => [m.folder, m.needs] as const));
  const buildTarget = (m: Mod): Target => ({
    kind: "mod",
    color: null,
    folder: m.name,
    label: m.title ?? m.name,
    id: m.id,
    nexusUrl: m.nexusUrl,
    installed: m.installed,
    enabled: m.enabled,
    optional: m.optional,
    removed: m.removed,
    buildId: null,
    needs: needs.get(m.name) ?? [],
  });
  const userTarget = (r: UserRow): Target =>
    r.kind === "mod"
      ? { kind: "mod", color: null, folder: r.name, label: r.name, id: null, nexusUrl: r.nexusUrl, installed: true, enabled: r.enabled, optional: false, removed: false, buildId: r.buildId, needs: needs.get(r.name) ?? [] }
      : { kind: "separator", color: r.color, folder: rowKey(r), label: r.title, id: null, nexusUrl: null, installed: true, enabled: true, optional: false, removed: false, buildId: null, needs: [] };
  const overwriteTarget: Target = { kind: "overwrite", color: null, folder: "overwrite", label: "Overwrite", id: null, nexusUrl: null, installed: true, enabled: true, optional: false, removed: false, buildId: null, needs: [] };
  const [query, setQuery] = useState("");
  const [filter, setFilter] = useState<Filter>("all");
  const [collapsed, setCollapsed] = useState<Set<string>>(loadCollapsed);
  const [menu, setMenu] = useState<Menu | null>(null);
  const listRef = useRef<HTMLDivElement>(null);
  const [current, setCurrent] = useState<string | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  // Like MO2's list: a click selects a mod, Ctrl adds or takes one away, Shift a range of the rows shown.
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const anchor = useRef<string | null>(null);
  const shownRows = () => [...(listRef.current?.querySelectorAll<HTMLElement>("[data-row]") ?? [])].map((el) => el.dataset.row!);
  const select = (e: MouseEvent, key: string) => {
    const add = e.ctrlKey || e.metaKey;
    const next = add ? new Set(selected) : new Set<string>();
    const rows = shownRows();
    const [a, b] = [rows.indexOf(anchor.current ?? ""), rows.indexOf(key)];
    if (e.shiftKey && a >= 0) {
      for (const k of rows.slice(Math.min(a, b), Math.max(a, b) + 1)) next.add(k);
    } else {
      if (!add || !next.delete(key)) next.add(key);
      anchor.current = key;
    }
    setSelected(next);
  };
  const openMenu = (e: MouseEvent, target: Target) => {
    e.preventDefault();
    // A right click outside the selection selects that row, as in MO2.
    if (target.kind === "mod" && !selected.has(target.folder)) {
      setSelected(new Set([target.folder]));
      anchor.current = target.folder;
    }
    setMenu({ target, x: e.clientX, y: e.clientY });
  };
  const toggleGroup = (title: string) => {
    const next = new Set(collapsed);
    if (!next.delete(title)) next.add(title);
    setCollapsed(next);
    saveCollapsed(next);
  };
  const setAllCollapsed = (titles: string[]) => {
    const next = new Set(titles);
    setCollapsed(next);
    saveCollapsed(next);
  };
  // The sidebar opens a folded group and scrolls it under the table header.
  const jump = (key: string) => {
    if (collapsed.has(key)) toggleGroup(key);
    requestAnimationFrame(() => {
      const list = listRef.current;
      const group = list?.querySelector<HTMLElement>(`tbody[data-group="${CSS.escape(key)}"]`);
      if (!list || !group) return;
      const top = list.scrollTop + group.getBoundingClientRect().top - list.getBoundingClientRect().top - HEADER_PX;
      list.scrollTo({ top, behavior: "smooth" });
    });
  };
  // The group under the table header is the one highlighted in the sidebar.
  const trackCurrent = () => {
    const list = listRef.current;
    if (!list) return;
    const edge = list.getBoundingClientRect().top + HEADER_PX + 1;
    let key: string | null = null;
    for (const g of list.querySelectorAll<HTMLElement>("tbody[data-group]")) {
      if (g.getBoundingClientRect().top > edge) break;
      key = g.dataset.group!;
    }
    setCurrent(key);
  };

  const isChange = (m: Mod) => !!m.recent || m.outdated || m.damaged || (!m.installed && !m.removed && !!build?.installedVersion);
  const isOff = (m: Mod) => !m.enabled || m.removed;
  const keep = (m: Mod) => filter === "all" || (filter === "off" ? isOff(m) : isChange(m));
  const keepUser = (r: UserMod) => filter === "all" || (filter === "off" && !r.enabled);
  const amongBuild = (userMods ?? []).filter((r): r is UserMod => r.kind === "mod" && r.after !== null);
  const groups = groupMods(build?.mods ?? [], amongBuild, query, keep, keepUser);
  const all = (build?.mods ?? []).filter((m): m is Mod => m.kind === "mod");
  const shown = groups.reduce((n, g) => n + g.mods.length, 0);
  const offCount = all.filter(isOff).length;
  const changeCount = all.filter(isChange).length;
  // MO2 rewrites modlist.txt on exit, so switching mods while it runs would be lost.
  const locked = !!progress || !!status?.gameRunning || !!status?.mo2Running;
  const lockReason = progress
    ? "Дождитесь окончания обновления"
    : "Закройте игру и Mod Organizer 2, чтобы включать и выключать моды";
  const targets = new Map<string, Target>([
    ...(build?.mods ?? []).flatMap((m) => (m.kind === "mod" ? [[m.name, buildTarget(m)] as const] : [])),
    ...(userMods ?? []).flatMap((r) => (r.kind === "mod" ? [[r.name, userTarget(r)] as const] : [])),
  ]);
  const picked = [...targets.values()].filter((t) => selected.has(t.folder));
  const author = !!settings?.authorMode;
  const switchable = (t: Target) => t.installed && !t.removed && (!t.id || t.optional);
  // The player's own, any in author mode; a player's optional build mod leaves the build (`remove_build_mod`).
  const deletable = (t: Target) => !t.id || author || (t.optional && t.installed && !t.removed);
  const switchMany = (ts: Target[], enabled: boolean) =>
    void run(async () => {
      for (const t of ts.filter((t) => switchable(t) && t.enabled !== enabled)) {
        if (t.id) await setModEnabled(t.id, enabled);
        else await api.setUserModEnabled(t.folder, enabled);
      }
      await refreshUserMods();
    });
  // Space switches the selection as its first mod goes, Delete removes it.
  const onKey = (e: React.KeyboardEvent) => {
    if (e.target !== e.currentTarget) return;
    if (e.key === "Escape") setSelected(new Set());
    else if (e.key === "a" && (e.ctrlKey || e.metaKey)) setSelected(new Set(shownRows()));
    else if (e.key === " " && picked.length > 0 && !locked) switchMany(picked, !picked[0].enabled);
    else if (e.key === "Delete" && !locked && picked.some(deletable)) {
      const many = picked.filter(deletable);
      setDialog({ kind: "delete", target: many[0], many });
    } else return;
    e.preventDefault();
  };

  // The player's own MO2: the whole list is their section.
  const isBuild = !!activeInstance(settings)?.build;
  if (!build && isBuild) {
    return (
      <div className="mx-auto max-w-[1600px]">
        <p className="text-muted">{buildError ?? "Загрузка списка модов…"}</p>
        <Notices />
      </div>
    );
  }
  const last = build?.lastUpdate ?? null;
  // Where an archive being dragged or dropped from Explorer, or a row of the player, would land: a mod anywhere,
  // among the build's too (updates keep it under the build row above it).
  const spot = drag?.over ?? (fileOver && !fileOver.outside ? fileOver : null);
  const dropping = !!drag || (!!fileOver && !fileOver.outside);
  // A separator of the player moves only within their section: MO2 would put build mods under it.
  const lineAfter = spot && !(drag?.move && movesSeparator(drag) && spot.build) && (spot.after === null ? "end" : spot.after);
  const userRows = (userMods ?? []).filter((r) => r.kind === "separator" || (r.after === null && keepUser(r)));
  const q = query.trim().toLowerCase();
  const shownUser = userRows.filter((r) => !q || r.kind === "separator" || r.name.toLowerCase().includes(q));
  const userModCount = (userMods ?? []).filter((r) => r.kind === "mod").length;
  // The player's separators fold their mods like the build's; a search or a filter hides the empty ones.
  const userGroups: UserGroup[] = [{ sep: null, mods: [] }];
  for (const r of shownUser) {
    if (r.kind === "separator") userGroups.push({ sep: r, mods: [] });
    else userGroups[userGroups.length - 1].mods.push(r);
  }
  const userSeps = userGroups.slice(1).filter((g) => g.mods.length > 0 || (!q && filter === "all"));
  // An archive from the downloads opens folded groups to land in; a moved row doesn't: the list would jump under the pointer.
  const expand = dropping && !drag?.move;
  const userOpen = !!q || expand || !collapsed.has(USER_SEPARATOR);
  const groupOpen = (key: string) => !!q || expand || !collapsed.has(key);
  const userModsShown = shownUser.filter((r) => r.kind === "mod").length;
  // The row a drop at the end of the list lands under: the last one shown.
  const lastUser = [userGroups[0], ...userSeps].reverse().find((g) => g.sep || g.mods.length > 0);
  const endKey = !lastUser
    ? USER_SEPARATOR
    : lastUser.sep && !groupOpen(rowKey(lastUser.sep))
      ? rowKey(lastUser.sep)
      : lastUser.mods.length > 0
        ? lastUser.mods[lastUser.mods.length - 1].name
        : rowKey(lastUser.sep!);
  // Like MO2, overwrite/ is the last row of the list: above every mod.
  const showOverwrite = !!overwrite && overwrite.files > 0 && filter === "all" && !q;
  const showUser = (shownUser.length > 0 || dropping || showOverwrite) && filter !== "changes";
  // A drag reorders the player's own rows; MO2 would overwrite the list on exit, so it waits for it to close.
  // A selected mod takes the player's other selected mods along (build mods keep their places); an unselected one becomes the selection.
  const startMove = (e: React.PointerEvent, target: Target) =>
    !locked &&
    pressToDrag(e, (ev) => {
      const together = target.kind === "mod" && selected.has(target.folder);
      const files = together ? (userMods ?? []).flatMap((r) => (r.kind === "mod" && selected.has(r.name) ? [r.name] : [])) : [target.folder];
      if (target.kind === "mod" && !together) {
        setSelected(new Set(files));
        anchor.current = target.folder;
      }
      const label = files.length > 1 ? `${files.length} ${plural(files.length, "мод", "мода", "модов")}` : target.label;
      useApp.setState({ drag: { file: target.folder, files, label, x: ev.clientX, y: ev.clientY, over: null, move: true } });
    });
  const userRow = (r: UserMod) => (
    <UserModRow
      key={r.name}
      row={r}
      build={r.after !== null}
      update={nexusUpdate.get(r.name)}
      needs={needs.get(r.name)}
      locked={locked}
      lockReason={lockReason}
      lineClass={line(r.name)}
      active={menu?.target.folder === r.name}
      selected={selected.has(r.name)}
      onSelect={(e) => select(e, r.name)}
      onContext={(e) => openMenu(e, userTarget(r))}
      onMenu={(x, y) => setMenu({ target: userTarget(r), x, y })}
      onPress={(e) => startMove(e, userTarget(r))}
    />
  );
  const nav = [
    ...groups.flatMap((g) => (g.title ? [{ key: g.title, title: g.title, color: g.color, count: g.mods.length }] : [])),
    ...(showUser ? [{ key: USER_SEPARATOR, title: isBuild ? "Мои моды" : "Моды", color: null, count: userModsShown }] : []),
    ...(showUser ? userSeps.map((g) => ({ key: rowKey(g.sep!), title: g.sep!.title, color: g.sep!.color, count: g.mods.length })) : []),
  ];
  const active = nav.some((n) => n.key === current) ? current : nav[0]?.key;
  const allFolded = nav.length > 0 && nav.every((n) => collapsed.has(n.key));
  const line = (key: string) => (lineAfter === key || (lineAfter === "end" && key === endKey) ? DROP_LINE : "");

  return (
    <div className="mx-auto flex h-full max-w-[1600px] flex-col">
      <div className="flex items-end justify-between gap-6">
        <PageTitle
          sub={
            build
              ? `${all.length} ${plural(all.length, "мод", "мода", "модов")} · сборка ${build.latestVersion}`
              : `${userModCount} ${plural(userModCount, "мод", "мода", "модов")} · Mod Organizer 2`
          }
        >
          Моды
        </PageTitle>
        <div className="flex items-center gap-4">
          <Button
            variant="ghost"
            disabled={!!nexusChecking || !nexus?.account}
            title={nexus?.account ? "Узнать на Nexus Mods, вышли ли новые версии модов" : "Войдите в Nexus Mods в настройках"}
            onClick={() => void checkNexus(false)}
          >
            <RefreshCw size={14} className={nexusChecking ? "animate-spin" : ""} />
            {nexusChecking ? `Проверка ${nexusChecking.done}/${nexusChecking.total}` : "Проверить обновления"}
          </Button>
          {(offCount > 0 || changeCount > 0) && (
            <div className="flex items-center gap-1.5">
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
          <label className="relative w-72">
            <Search size={15} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-faint" />
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              placeholder="Поиск по названию"
              spellCheck={false}
              className="h-9 w-full rounded-full bg-surface pr-4 pl-9 text-sm outline-none placeholder:text-faint focus:ring-1 focus:ring-neon/50"
            />
          </label>
        </div>
      </div>

      <Notices />
      {filter === "off" && (
        <p className="mt-3 text-[13px] text-muted">
          Выбор сохраняется при обновлениях сборки. Выключенный мод остаётся установленным, удалённый обновления не
          вернут, пока вы сами не вернёте его через меню мода. Если от мода зависят другие, выключите и их: лаунчер
          зависимостей не проверяет.
        </p>
      )}
      {filter === "changes" && last && last.removed.length > 0 && (
        <p className="mt-3 text-[13px] text-muted">
          Удалены в {last.to}: {last.removed.join(", ")}
        </p>
      )}

      <div className="mt-4 flex min-h-0 flex-1 gap-4">
        <aside className="panel flex w-80 shrink-0 flex-col overflow-hidden">
          <div className="label flex h-9 shrink-0 items-center justify-between border-b border-line pr-2 pl-4">
            <span className="font-medium">Категории</span>
            {nav.length > 0 && (
              <button
                onClick={() => setAllCollapsed(allFolded ? [] : nav.map((n) => n.key))}
                className="rounded-full px-2 py-0.5 text-[12px] text-muted transition-colors hover:bg-raised hover:text-fg"
              >
                {allFolded ? "Развернуть все" : "Свернуть все"}
              </button>
            )}
          </div>
          <nav className="min-h-0 flex-1 overflow-y-auto py-1.5">
            {nav.map((n) => (
              <button
                key={n.key}
                onClick={() => jump(n.key)}
                title={n.title}
                className={`flex h-8 w-full items-center gap-2.5 px-4 text-left text-[13px] transition-colors hover:bg-raised/60 hover:text-fg ${
                  active === n.key ? "bg-raised text-fg" : "text-muted"
                } ${collapsed.has(n.key) && !q ? "opacity-60" : ""}`}
              >
                <span className="h-3.5 w-1 shrink-0 rounded-full" style={{ background: n.color ?? "var(--color-line)" }} />
                <span className="min-w-0 flex-1 truncate">{n.title}</span>
                <span className="font-mono text-[11px] text-faint tabular-nums">{n.count}</span>
              </button>
            ))}
          </nav>
          <p className="shrink-0 border-t border-line px-4 py-3 text-[11px] leading-relaxed text-faint">
            Все моды принадлежат их авторам. Если мод понравился — поддержите автора на Nexus Mods.
          </p>
        </aside>

        <div
          ref={listRef}
          onScroll={trackCurrent}
          onKeyDown={onKey}
          tabIndex={0}
          data-drop-zone
          className={`panel min-h-0 flex-1 overflow-y-auto outline-none transition-shadow ${dropping ? "ring-1 ring-neon/40" : ""}`}
        >
          <table className="w-full table-fixed text-left select-none">
            <colgroup>
              <col />
              <col className="w-32" />
              <col className="w-24" />
            </colgroup>
            <thead className="sticky top-0 z-10 bg-surface">
              <tr className="label h-9 border-b border-line">
                <th className="pl-5 font-medium">Название</th>
                <th className="font-medium">Версия</th>
                <th className="pr-5" />
              </tr>
            </thead>
            {groups.map((g, i) => (
              <tbody key={g.title ?? `group-${i}`} data-group={g.title ?? undefined}>
                {g.title && (
                  <SeparatorRow
                    build
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
                {(!g.title || query.trim() || !collapsed.has(g.title)) && g.mods.map((m) => !isBuildMod(m) ? userRow(m) : (
                  <tr
                    key={m.id}
                    data-drop-after={m.name}
                    data-drop-build
                    data-row={m.name}
                    onClick={(e) => select(e, m.name)}
                    onContextMenu={(e) => openMenu(e, buildTarget(m))}
                    className={`h-10 cursor-pointer border-b border-line/60 last:border-b-0 ${rowTone(selected.has(m.name), menu?.target.folder === m.name)} ${line(m.name)}`}
                  >
                    <td className="pl-5">
                     <div className="flex min-w-0 items-center gap-2">
                      <span className={`truncate ${m.enabled && !m.removed ? "" : "text-faint"}`}>{m.title ?? m.name}</span>
                      <span className="flex shrink-0 items-center gap-1">
                      {m.removed && <Badge tone="state" title="Вы удалили этот мод: обновления сборки его не вернут">удалён</Badge>}
                      {!m.enabled && !m.removed && <Badge tone="state">выключен</Badge>}
                      {m.recent && last && (
                        <Badge tone="info" icon={m.recent === "added" ? Plus : ArrowUp} title={`В версии ${last.to}`}>
                          {m.recent === "added" ? "добавлен" : "обновлён"}
                        </Badge>
                      )}
                      {m.outdated && <Badge tone="pending" icon={ArrowUp}>обновится</Badge>}
                      {m.damaged && !m.outdated && (
                        <Badge tone="pending" icon={Wrench} title="Проверка нашла изменённые или удалённые файлы">
                          восстановится
                        </Badge>
                      )}
                      {!m.installed && !m.removed && build?.installedVersion && <Badge tone="pending" icon={Plus}>новый</Badge>}
                      <NeedsBadge needs={needs.get(m.name)} />
                      </span>
                     </div>
                    </td>
                    <VersionCell version={m.version} update={nexusUpdate.get(m.name)} />
                    <td className="pr-5">
                      <div className="flex items-center justify-end gap-1" onClick={(e) => e.stopPropagation()}>
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
                        <MenuButton onOpen={(x, y) => setMenu({ target: buildTarget(m), x, y })} />
                      </div>
                    </td>
                  </tr>
                ))}
              </tbody>
            ))}
            {showUser && (
              <tbody data-group={USER_SEPARATOR}>
                <SeparatorRow
                  title={isBuild ? "Мои моды" : "Моды"}
                  color={null}
                  count={userModsShown}
                  open={userOpen}
                  onToggle={() => toggleGroup(USER_SEPARATOR)}
                  dropKey={USER_SEPARATOR}
                  lineClass={line(USER_SEPARATOR)}
                />
                {userOpen && userGroups[0].mods.map(userRow)}
                {dropping && shownUser.length === 0 && !drag?.move && (
                  <tr data-drop-after={USER_SEPARATOR}>
                    <td colSpan={3} className="px-5 py-4 text-center text-[13px] text-neon">
                      Отпустите архив, чтобы установить его в ваши моды
                    </td>
                  </tr>
                )}
              </tbody>
            )}
            {showUser &&
              userOpen &&
              userSeps.map((g) => {
                const key = rowKey(g.sep!);
                const target = userTarget(g.sep!);
                return (
                  <tbody key={key} data-group={key}>
                    <SeparatorRow
                      title={g.sep!.title}
                      color={g.sep!.color}
                      count={g.mods.length}
                      open={groupOpen(key)}
                      onToggle={() => toggleGroup(key)}
                      dropKey={key}
                      lineClass={line(key)}
                      active={menu?.target.folder === key}
                      onMenu={(x, y) => setMenu({ target, x, y })}
                      onPress={(e) => startMove(e, target)}
                    />
                    {groupOpen(key) && g.mods.map(userRow)}
                  </tbody>
                );
              })}
            {showUser && userOpen && showOverwrite && (
              <tbody>
                <tr
                  onClick={(e) => openMenu(e, overwriteTarget)}
                  onContextMenu={(e) => openMenu(e, overwriteTarget)}
                  className={`h-10 cursor-pointer hover:bg-raised/50 ${menu?.target.kind === "overwrite" ? "bg-raised/50" : ""}`}
                  title="Файлы, которые игра и моды создали через Mod Organizer 2: настройки CET, логи, кэш. Они важнее любого мода."
                >
                  <td className="pl-5">
                    <div className="flex min-w-0 items-center gap-2">
                      <span className="truncate text-muted">Overwrite</span>
                      <Badge tone="state">{`${overwrite!.files} ${plural(overwrite!.files, "файл", "файла", "файлов")} · ${formatBytes(overwrite!.size)}`}</Badge>
                    </div>
                  </td>
                  <td />
                  <td className="pr-5">
                    <div className="flex items-center justify-end gap-1" onClick={(e) => e.stopPropagation()}>
                      <MenuButton onOpen={(x, y) => setMenu({ target: overwriteTarget, x, y })} />
                    </div>
                  </td>
                </tr>
              </tbody>
            )}
          </table>
          {shown === 0 && shownUser.length === 0 && !dropping && <p className="px-5 py-8 text-center text-[13px] text-muted">Ничего не найдено</p>}
        </div>
      </div>

      {menu && (
        <ModMenu
          menu={menu}
          many={menu.target.kind === "mod" && picked.length > 1 && selected.has(menu.target.folder) ? picked : null}
          switchable={switchable}
          deletable={deletable}
          onSwitch={switchMany}
          locked={locked}
          lockReason={lockReason}
          onDialog={setDialog}
          onClose={() => setMenu(null)}
        />
      )}
      {dialog?.kind === "logs" ? <LogViewer onClose={() => setDialog(null)} /> : dialog && <ModDialog dialog={dialog} onClose={() => setDialog(null)} />}
    </div>
  );
}

function MenuButton({ onOpen }: { onOpen: (x: number, y: number) => void }) {
  return (
    <button
      onClick={(e) => {
        const r = e.currentTarget.getBoundingClientRect();
        onOpen(r.right, r.bottom + 4);
      }}
      className="inline-flex h-7 w-7 items-center justify-center rounded-full text-muted transition-colors hover:bg-raised hover:text-fg"
      title="Действия с модом"
      aria-haspopup="menu"
    >
      <MoreHorizontal size={16} />
    </button>
  );
}

type Item = { label: string; icon: LucideIcon; onClick: () => void; disabled?: boolean; title?: string; danger?: boolean };

// Like MO2's right-click menu on a mod. Deleting and renaming are for the player's own mods: a build mod
// would come back with the next update, so the player only switches it off (the author changes any).
function ModMenu(props: {
  menu: Menu;
  /** Several selected mods: the menu acts on all of them. */
  many: Target[] | null;
  switchable: (t: Target) => boolean;
  deletable: (t: Target) => boolean;
  onSwitch: (ts: Target[], enabled: boolean) => void;
  locked: boolean;
  lockReason: string;
  onDialog: (d: Dialog) => void;
  onClose: () => void;
}) {
  const { run, settings, userMods, refreshBuild, refreshUserMods } = useApp();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: props.menu.x, top: props.menu.y });
  const t = props.menu.target;
  const author = !!settings?.authorMode;
  const own = !t.id || author;
  // A player's optional build mod goes for good: updates leave it out (`remove_build_mod`).
  const removable = own || (t.optional && t.installed);
  const dialog = (kind: Dialog["kind"], after?: string | null) => () => props.onDialog({ kind, target: t, after });
  // A player's build mod comes again with the build (repair), which needs MO2 closed; an archive install waits for it.
  const repair = !!t.id && !author;
  const lock = props.locked ? props.lockReason : undefined;
  // Above the mod, as MO2's "Create separator": below the row before it, or at the top of the section.
  // Not among the build's: MO2 would put build mods under it.
  const section = (userMods ?? []).filter((r) => r.kind === "separator" || r.after === null);
  const at = section.findIndex((r) => rowKey(r) === t.folder);
  const above = at > 0 ? rowKey(section[at - 1]) : USER_SEPARATOR;
  const many = props.many;
  const off = many?.filter((m) => props.switchable(m) && !m.enabled).length ?? 0;
  const on = many?.filter((m) => props.switchable(m) && m.enabled).length ?? 0;
  const gone = many?.filter(props.deletable) ?? [];
  const items: (Item | null)[] = many ? [
    { label: `Включить (${off})`, icon: Power, disabled: off === 0 || props.locked, title: lock, onClick: () => props.onSwitch(many, true) },
    { label: `Выключить (${on})`, icon: PowerOff, disabled: on === 0 || props.locked, title: lock, onClick: () => props.onSwitch(many, false) },
    null,
    {
      label: `Удалить (${gone.length})`,
      icon: Trash2,
      danger: true,
      disabled: gone.length === 0 || props.locked,
      title: lock ?? (gone.length < many.length ? "Основу сборки удалить нельзя" : undefined),
      onClick: () => props.onDialog({ kind: "delete", target: gone[0], many: gone }),
    },
  ] : t.kind === "separator" ? [
    { label: "Переименовать", icon: Pencil, disabled: props.locked, title: lock, onClick: dialog("rename") },
    { label: "Цвет", icon: Palette, disabled: props.locked, title: lock, onClick: dialog("color") },
    { label: "Удалить", icon: Trash2, danger: true, disabled: props.locked, title: lock, onClick: dialog("delete") },
  ] : t.kind === "overwrite" ? [
    { label: "Логи игры", icon: ScrollText, onClick: dialog("logs") },
    { label: "Открыть в проводнике", icon: FolderOpen, onClick: () => run(() => api.openFolder("overwrite")) },
    null,
    { label: "Сделать модом", icon: PackagePlus, disabled: props.locked, title: lock, onClick: dialog("toMod") },
    { label: "Очистить", icon: Eraser, danger: true, disabled: props.locked, title: lock, onClick: dialog("clear") },
  ] : [
    {
      label: "Открыть в проводнике",
      icon: FolderOpen,
      disabled: !t.installed,
      title: t.installed ? undefined : "Мод ещё не установлен",
      onClick: () => run(() => (t.id ? api.openModFolder(t.id) : api.openUserModFolder(t.folder))),
    },
    {
      label: "Открыть на Nexus",
      icon: ExternalLink,
      disabled: !t.nexusUrl,
      title: t.nexusUrl ?? "У мода нет страницы на Nexus",
      onClick: () => run(() => api.openUrl(t.nexusUrl!)),
    },
    null,
    {
      label: "Переустановить",
      icon: RefreshCw,
      disabled: !t.installed || (repair && props.locked),
      title: !t.installed ? "Мод ещё не установлен" : repair && props.locked ? props.lockReason : undefined,
      onClick: dialog("reinstall"),
    },
    {
      label: "Переименовать",
      icon: Pencil,
      disabled: !own || props.locked,
      title: !own ? "Мод сборки можно только выключить" : props.locked ? props.lockReason : undefined,
      onClick: dialog("rename"),
    },
    ...(t.buildId
      ? [
          {
            label: "Вернуть версию сборки",
            icon: Undo2,
            disabled: props.locked,
            title: props.locked ? props.lockReason : "Версия сборки скачается со следующим обновлением и заменит вашу",
            onClick: () =>
              void run(async () => {
                await api.restoreBuildMod(t.buildId!);
                await Promise.all([refreshBuild(), refreshUserMods()]);
              }),
          },
        ]
      : []),
    t.removed
      ? {
          label: "Вернуть в сборку",
          icon: Undo2,
          disabled: props.locked,
          title: props.locked ? props.lockReason : "Мод установится со следующим обновлением сборки",
          onClick: () =>
            void run(async () => {
              await api.restoreBuildMod(t.id!);
              await refreshBuild();
            }),
        }
      : {
          label: "Удалить",
          icon: Trash2,
          danger: true,
          disabled: !removable || props.locked,
          title: !removable ? (t.installed ? "Основа сборки: от этого мода зависят другие, удалить его нельзя" : "Мод ещё не установлен") : props.locked ? props.lockReason : undefined,
          onClick: dialog("delete"),
        },
    ...(t.id || at < 0
      ? []
      : [null, { label: "Разделитель над модом", icon: SeparatorHorizontal, disabled: props.locked, title: lock, onClick: dialog("separator", above) }]),
    // What its Nexus page asks for and the list lacks: a way to the page, the install stays the player's drag.
    ...(t.needs.length > 0 ? [null] : []),
    ...t.needs.map((n) => ({
      label: n.disabled ? `${n.name} — выключен` : n.modId === null ? `Требует: ${n.name}` : `Нет мода: ${n.name}`,
      icon: ExternalLink,
      disabled: !n.url,
      title: [n.disabled && `Включите «${n.disabled}»`, n.notes, n.url].filter(Boolean).join("\n") || undefined,
      onClick: () => run(() => api.openUrl(n.url)),
    })),
  ];

  // Opened near an edge, the menu flips inside the window.
  useLayoutEffect(() => {
    const r = ref.current!.getBoundingClientRect();
    setPos({
      left: Math.max(8, Math.min(props.menu.x, window.innerWidth - r.width - 8)),
      top: props.menu.y + r.height > window.innerHeight - 8 ? Math.max(8, props.menu.y - r.height) : props.menu.y,
    });
    ref.current!.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
  }, [props.menu]);

  useEffect(() => {
    const outside = (e: Event) => !ref.current?.contains(e.target as Node) && props.onClose();
    const key = (e: KeyboardEvent) => {
      if (e.key === "Escape") props.onClose();
      if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
      e.preventDefault();
      const buttons = [...ref.current!.querySelectorAll<HTMLButtonElement>("button:not(:disabled)")];
      const at = buttons.indexOf(document.activeElement as HTMLButtonElement);
      buttons[(at + (e.key === "ArrowDown" ? 1 : buttons.length - 1)) % buttons.length]?.focus();
    };
    window.addEventListener("mousedown", outside);
    window.addEventListener("contextmenu", outside, true);
    window.addEventListener("scroll", props.onClose, true);
    window.addEventListener("resize", props.onClose);
    window.addEventListener("blur", props.onClose);
    window.addEventListener("keydown", key);
    return () => {
      window.removeEventListener("mousedown", outside);
      window.removeEventListener("contextmenu", outside, true);
      window.removeEventListener("scroll", props.onClose, true);
      window.removeEventListener("resize", props.onClose);
      window.removeEventListener("blur", props.onClose);
      window.removeEventListener("keydown", key);
    };
  }, [props.onClose]);

  return (
    <div
      ref={ref}
      role="menu"
      style={pos}
      onContextMenu={(e) => e.preventDefault()}
      className="panel fixed z-50 w-60 bg-surface py-1.5 shadow-2xl"
    >
      <div className="truncate px-3.5 pt-1 pb-2 text-[12px] font-semibold text-muted">
        {many ? `Выбрано: ${many.length} ${plural(many.length, "мод", "мода", "модов")}` : t.label}
      </div>
      {items.map((it, i) =>
        it === null ? (
          <div key={i} className="my-1.5 border-t border-line" />
        ) : (
          <button
            key={`${i}-${it.label}`}
            role="menuitem"
            disabled={it.disabled}
            title={it.title}
            onClick={() => {
              props.onClose();
              it.onClick();
            }}
            className={`flex h-8 w-full items-center gap-2.5 px-3.5 text-left text-[13px] transition-colors outline-none disabled:cursor-not-allowed disabled:text-faint ${
              it.danger ? "text-bad hover:bg-bad/10 focus:bg-bad/10" : "text-fg hover:bg-raised focus:bg-raised"
            }`}
          >
            <it.icon size={14} className="shrink-0 opacity-80" />
            <span className="truncate">{it.label}</span>
          </button>
        ),
      )}
    </div>
  );
}

function ModDialog({ dialog, onClose }: { dialog: Dialog; onClose: () => void }) {
  const { run, settings, refreshUserMods, refreshBuild, startRepair } = useApp();
  const t = dialog.target;
  const initial = { rename: t.kind === "separator" ? t.label : t.folder, separator: "", toMod: "Overwrite" }[dialog.kind as string] ?? "";
  const [name, setName] = useState(initial);
  const [color, setColor] = useState(t.color ?? "#3cf2a0");
  const repair = !!t.id && !settings?.authorMode;

  useEffect(() => {
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, []);

  const act = (action: () => Promise<unknown>) => {
    onClose();
    void run(async () => {
      await action();
      await Promise.all([refreshUserMods(), t.id ? refreshBuild() : null]);
    });
  };
  const newName = name.trim();
  const named = dialog.kind === "rename" || dialog.kind === "separator" || dialog.kind === "toMod";
  const submit = () => {
    if (!newName || newName === initial) return;
    if (dialog.kind === "rename") act(() => api.renameMod(t.folder, newName));
    if (dialog.kind === "separator") act(() => api.addSeparator(newName, dialog.after ?? null));
    if (dialog.kind === "toMod") act(() => api.overwriteToMod(newName));
  };
  const nameInput = (
    <input
      autoFocus
      value={name}
      onChange={(e) => setName(e.target.value)}
      onFocus={(e) => e.target.select()}
      onKeyDown={(e) => e.key === "Enter" && submit()}
      placeholder={dialog.kind === "separator" ? "Название разделителя" : undefined}
      spellCheck={false}
      className="h-9 w-full rounded-lg bg-raised px-3 text-sm text-fg outline-none placeholder:text-faint focus:ring-1 focus:ring-neon/50"
    />
  );

  const view: Record<Exclude<Dialog["kind"], "logs">, { title: string; body: ReactNode; confirm: string; danger?: boolean; ok: () => void }> = {
    delete: t.kind === "separator" ? {
      title: "Удалить разделитель?",
      body: <p>Моды под ним останутся на своих местах.</p>,
      confirm: "Удалить",
      danger: true,
      ok: () => act(() => api.deleteMod(t.folder)),
    } : repair ? {
      title: "Удалить мод сборки?",
      body: (
        <p>
          Папка мода удалится с диска, мод пропадёт из списка, и обновления сборки его не вернут. Передумаете — «Вернуть в сборку»
          в меню мода, и он скачается заново со следующим обновлением. Если от мода зависят другие, удалите или выключите и их.
        </p>
      ),
      confirm: "Удалить",
      danger: true,
      ok: () => act(() => api.removeBuildMod(t.id!)),
    } : {
      title: "Удалить мод?",
      body: <p>Папка мода удалится с диска вместе со всеми файлами, мод пропадёт из списка во всех профилях. Отменить это нельзя.</p>,
      confirm: "Удалить",
      danger: true,
      ok: () => act(() => api.deleteMod(t.folder)),
    },
    reinstall: {
      title: "Переустановить мод?",
      body: repair ? (
        <p>Мод скачается заново из сборки. Изменённые вами файлы настроек сохранятся.</p>
      ) : (
        <p>Мод установится заново из своего архива в загрузках Mod Organizer 2. Файлы мода заменятся целиком, место в списке и включённость сохранятся.</p>
      ),
      confirm: "Переустановить",
      ok: () => {
        if (!repair) return act(() => api.reinstallMod(t.folder));
        onClose();
        void startRepair([t.id!], false, false);
      },
    },
    rename: { title: t.kind === "separator" ? "Переименовать разделитель" : "Переименовать мод", body: nameInput, confirm: "Переименовать", ok: submit },
    separator: { title: "Новый разделитель", body: nameInput, confirm: "Добавить", ok: submit },
    toMod: {
      title: "Сделать модом",
      body: (
        <>
          <p>Файлы из overwrite станут отдельным модом в конце вашего списка: их можно будет выключить или удалить как обычный мод.</p>
          {nameInput}
        </>
      ),
      confirm: "Создать мод",
      ok: submit,
    },
    clear: {
      title: "Очистить overwrite?",
      body: (
        <p>
          Удалятся все файлы, которые игра и моды создали через Mod Organizer 2: настройки CET и других модов, логи, кэш.
          Моды создадут их заново с настройками по умолчанию. Отменить это нельзя.
        </p>
      ),
      confirm: "Очистить",
      danger: true,
      ok: () => act(() => api.clearOverwrite()),
    },
    color: {
      title: "Цвет разделителя",
      body: (
        <div className="flex items-center gap-3">
          <input type="color" value={color} onChange={(e) => setColor(e.target.value)} className="h-9 w-14 cursor-pointer rounded-lg bg-raised" />
          <span className="font-mono text-xs">{color}</span>
          {t.color && (
            <button onClick={() => act(() => api.setSeparatorColor(t.folder, null))} className="ml-auto text-[13px] text-muted hover:text-fg">
              Убрать цвет
            </button>
          )}
        </div>
      ),
      confirm: "Сохранить",
      ok: () => act(() => api.setSeparatorColor(t.folder, color)),
    },
  };
  const many = dialog.many && dialog.many.length > 1 ? dialog.many : null;
  if (many) {
    // A player's build mods leave the build, the rest goes from the disk.
    const fromBuild = (m: Target) => !!m.id && !settings?.authorMode;
    view.delete = {
      title: `Удалить ${many.length} ${plural(many.length, "мод", "мода", "модов")}?`,
      body: (
        <>
          <p>
            Папки модов удалятся с диска. Моды сборки обновления не вернут, пока вы не вернёте их через меню мода; ваши моды не
            вернуть.
          </p>
          <p className="max-h-40 overflow-y-auto font-mono text-xs text-faint">{many.map((m) => m.label).join("\n")}</p>
        </>
      ),
      confirm: "Удалить",
      danger: true,
      ok: () => {
        onClose();
        void run(async () => {
          for (const m of many) await (fromBuild(m) ? api.removeBuildMod(m.id!) : api.deleteMod(m.folder));
          await Promise.all([refreshUserMods(), refreshBuild()]);
        });
      },
    };
  }
  const v = view[dialog.kind as Exclude<Dialog["kind"], "logs">];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-8" role="dialog" aria-modal="true" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="panel flex w-full max-w-md flex-col overflow-hidden bg-surface shadow-2xl">
        <div className="flex h-14 shrink-0 items-center gap-3 border-b border-line px-5">
          <div className="min-w-0 flex-1">
            <div className="text-sm font-semibold">{v.title}</div>
            <div className="truncate text-xs text-muted">{t.label}</div>
          </div>
          <button onClick={onClose} aria-label="Отмена" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <X size={16} />
          </button>
        </div>
        <div className="space-y-2 px-5 py-4 text-[13px] text-muted">{v.body}</div>
        <div className="flex justify-end gap-2 border-t border-line px-5 py-3">
          <Button variant="ghost" onClick={onClose}>
            Отмена
          </Button>
          <Button
            variant="primary"
            autoFocus={!named}
            disabled={named && (!newName || newName === initial)}
            onClick={v.ok}
            className={v.danger ? "!bg-bad !text-fg !shadow-none" : ""}
          >
            {v.confirm}
          </Button>
        </div>
      </div>
    </div>
  );
}

// By kind: state is plain text, info (what the last build changed) is tinted, pending (what the next
// update will do) is outlined, problem (needs the player's action) is solid.
// An insertion line on the row's lower edge with a dot at its start. Drawn on the cells: a <tr> can't hold positioned
// children. A sticky separator cell keeps `sticky` (it holds the line as well): turned `relative`, it would move away
// from under the pointer, the line would go, the cell come back, and so on with every pointer move.
const DROP_LINE =
  "[&>td:not(.sticky)]:relative [&>td]:after:pointer-events-none [&>td]:after:absolute [&>td]:after:inset-x-0 [&>td]:after:-bottom-px [&>td]:after:z-10 [&>td]:after:h-0.5 [&>td]:after:bg-neon [&>td]:after:shadow-[0_0_8px_var(--color-neon)] " +
  "[&>td:first-child]:before:pointer-events-none [&>td:first-child]:before:absolute [&>td:first-child]:before:bottom-[-4px] [&>td:first-child]:before:left-2 [&>td:first-child]:before:z-20 [&>td:first-child]:before:size-2 [&>td:first-child]:before:rounded-full [&>td:first-child]:before:bg-neon";

const badgeTones = {
  state: "text-faint",
  info: "bg-ok/10 text-ok",
  pending: "text-muted ring-1 ring-inset ring-line",
  problem: "bg-warn/15 font-medium text-warn",
};

function Badge(props: { children: string; tone: keyof typeof badgeTones; icon?: LucideIcon; title?: string }) {
  const Icon = props.icon;
  return (
    <span
      title={props.title}
      className={`inline-flex items-center gap-1 whitespace-nowrap rounded-full text-[11px] leading-4 ${props.tone === "state" ? "" : "px-2 py-0.5"} ${badgeTones[props.tone]}`}
    >
      {Icon && <Icon size={11} />}
      {props.children}
    </span>
  );
}

// Only what the launcher can check: links outside Nexus (ReShade, a site) stay in the mod's menu.
function NeedsBadge({ needs }: { needs?: Need[] }) {
  const missing = (needs ?? []).filter((n) => n.modId !== null);
  if (missing.length === 0) return null;
  const lines = missing.map((n) => (n.disabled ? `${n.name} — выключен` : `${n.name} — не установлен`));
  return (
    <Badge tone="problem" icon={TriangleAlert} title={`Требуется на странице Nexus:\n${lines.join("\n")}\n\nСсылки — в меню мода`}>
      {`не хватает ${missing.length} ${plural(missing.length, "мода", "модов", "модов")}`}
    </Badge>
  );
}

type NexusUpdate = { version: string | null; url: string };

// An outdated version is a link to the mod's files on Nexus; installing the download is the player's drag.
function VersionCell({ version, update }: { version: string | null; update?: NexusUpdate }) {
  const { run } = useApp();
  const [tip, setTip] = useState<{ left: number; top: number } | null>(null);
  // Fixed, so the cell's truncate and the list's scroll don't clip it; scrolling would leave it behind.
  useEffect(() => {
    if (!tip) return;
    const hide = () => setTip(null);
    window.addEventListener("scroll", hide, true);
    return () => window.removeEventListener("scroll", hide, true);
  }, [tip]);
  if (!update) return <td className="truncate font-mono text-xs text-muted tabular-nums">{version ?? "—"}</td>;
  const show = (e: { currentTarget: HTMLElement }) => {
    const r = e.currentTarget.getBoundingClientRect();
    setTip({ left: r.left, top: r.bottom + 6 });
  };
  return (
    <td className="truncate" onClick={(e) => e.stopPropagation()}>
      <button
        onClick={() => run(() => api.openUrl(update.url))}
        onMouseEnter={show}
        onMouseLeave={() => setTip(null)}
        onFocus={show}
        onBlur={() => setTip(null)}
        aria-label={`Новая версия на Nexus: ${update.version ?? "без номера"}`}
        className="rounded-full bg-warn/10 px-2 py-0.5 font-mono text-xs text-warn tabular-nums transition-colors outline-none hover:bg-warn/20 focus-visible:ring-1 focus-visible:ring-warn/50"
      >
        {version ?? "—"}
      </button>
      {tip && (
        <div role="tooltip" style={tip} className="panel pointer-events-none fixed z-50 bg-surface px-3.5 py-2.5 shadow-2xl">
          <div className="flex items-center gap-2 text-[13px]">
            <span className="text-muted">Новая версия</span>
            <span className="font-mono text-xs text-warn tabular-nums">{update.version ?? "без номера"}</span>
          </div>
          <div className="mt-1 flex items-center gap-1.5 text-[11px] text-faint">
            <ExternalLink size={11} />
            Нажмите, чтобы открыть на Nexus Mods
          </div>
        </div>
      )}
    </td>
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

function rowTone(selected: boolean, active: boolean): string {
  return selected ? "bg-neon/10 hover:bg-neon/15" : active ? "bg-raised/50" : "hover:bg-raised/50";
}

function rowKey(r: UserRow): string {
  return r.kind === "mod" ? r.name : `${r.title}_separator`;
}

// The player's own mods: theirs to switch, in MO2's list order, in their section or among the build's. Updates of
// the build never touch them.
function UserModRow(props: {
  row: UserMod;
  /** Among the build's: a separator of the player doesn't land on it. */
  build: boolean;
  update?: NexusUpdate;
  needs?: Need[];
  locked: boolean;
  lockReason: string;
  lineClass: string;
  active: boolean;
  selected: boolean;
  onSelect: (e: MouseEvent) => void;
  onContext: (e: MouseEvent) => void;
  onMenu: (x: number, y: number) => void;
  onPress: (e: React.PointerEvent) => void;
}) {
  const { run, refreshUserMods } = useApp();
  const r = props.row;
  const toggle = (enabled: boolean) =>
    run(async () => {
      await api.setUserModEnabled(r.name, enabled);
      await refreshUserMods();
    });
  return (
    <tr
      data-drop-after={r.name}
      data-drop-build={props.build || undefined}
      data-row={r.name}
      onPointerDown={props.onPress}
      onClick={props.onSelect}
      onContextMenu={props.onContext}
      className={`h-10 cursor-pointer border-b border-line/60 last:border-b-0 ${rowTone(props.selected, props.active)} ${props.lineClass}`}
    >
      <td className="pl-5">
        <div className="flex min-w-0 items-center gap-2">
          <span className={`truncate ${r.enabled ? "" : "text-faint"}`}>{r.name}</span>
          <span className="flex shrink-0 items-center gap-1">
            {r.buildId && (
              <Badge tone="info" title="Ваша версия мода сборки: обновления сборки её не трогают. Вернуть версию сборки — в меню мода">
                своя версия
              </Badge>
            )}
            {!r.enabled && <Badge tone="state">выключен</Badge>}
            <NeedsBadge needs={props.needs} />
          </span>
        </div>
      </td>
      <VersionCell version={r.version} update={props.update} />
      <td className="pr-5">
        <div className="flex items-center justify-end gap-1" onClick={(e) => e.stopPropagation()} onPointerDown={(e) => e.stopPropagation()}>
          <Switch
            checked={r.enabled}
            disabled={props.locked}
            title={props.locked ? props.lockReason : r.enabled ? "Выключить мод" : "Включить мод"}
            onChange={toggle}
          />
          <MenuButton onOpen={props.onMenu} />
        </div>
      </td>
    </tr>
  );
}

// Separators of the build come from the author's MO2 (title, order and color), the player's from theirs; like in MO2,
// a click folds the group. The player's have a menu and are dragged like their mods.
function SeparatorRow(props: {
  title: string;
  color: string | null;
  count: number;
  open: boolean;
  onToggle: () => void;
  dropKey?: string;
  lineClass?: string;
  /** One of the build's: nothing of the player's section lands on it. */
  build?: boolean;
  active?: boolean;
  onMenu?: (x: number, y: number) => void;
  onPress?: (e: React.PointerEvent) => void;
}) {
  // Opaque background: the row sticks under the table header while its group scrolls by.
  const style: CSSProperties = props.color
    ? {
        background: `linear-gradient(color-mix(in srgb, ${props.color} 14%, transparent), color-mix(in srgb, ${props.color} 14%, transparent)), var(--color-surface)`,
        color: `color-mix(in srgb, ${props.color} 55%, var(--color-fg))`,
      }
    : { background: "var(--color-raised)" };
  const onMenu = props.onMenu;
  return (
    <tr data-drop-after={props.dropKey} data-drop-build={props.build || undefined} className={props.lineClass}>
      <td colSpan={3} className="sticky top-9 z-[5] p-0">
        <div
          style={style}
          onPointerDown={props.onPress}
          onContextMenu={
            onMenu &&
            ((e) => {
              e.preventDefault();
              onMenu(e.clientX, e.clientY);
            })
          }
          className={`flex h-9 items-center pr-5 transition-[filter] hover:brightness-125 ${props.active ? "brightness-125" : ""}`}
        >
          <button
            onClick={props.onToggle}
            aria-expanded={props.open}
            className="flex h-full min-w-0 flex-1 items-center gap-2 pl-4 text-left text-[13px] font-semibold"
          >
            <ChevronRight size={14} className={`shrink-0 opacity-70 transition-transform ${props.open ? "rotate-90" : ""}`} />
            <span className="truncate">{props.title}</span>
            <span className="font-mono text-[11px] font-normal opacity-60 tabular-nums">{props.count}</span>
          </button>
          {onMenu && (
            <span onPointerDown={(e) => e.stopPropagation()}>
              <MenuButton onOpen={onMenu} />
            </span>
          )}
        </div>
      </td>
    </tr>
  );
}

const COLLAPSED_KEY = "mods.collapsed";
/** Height of the sticky table header (`h-9`): separators stick under it, jumps land under it. */
const HEADER_PX = 36;

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

function isBuildMod(m: Mod | UserMod): m is Mod {
  return "id" in m;
}

/** Build rows by separator, each followed by the player's mods placed under it (`after`). */
function groupMods(rows: ModRow[], user: UserMod[], query: string, keep: (m: Mod) => boolean, keepUser: (m: UserMod) => boolean): Group[] {
  const q = query.trim().toLowerCase();
  const matches = (m: Mod | UserMod) =>
    (isBuildMod(m) ? keep(m) : keepUser(m)) && (!q || [m.name, isBuildMod(m) ? m.title : null].some((s) => s?.toLowerCase().includes(q)));
  const under = new Map<string, UserMod[]>();
  for (const u of user) under.set(u.after!, [...(under.get(u.after!) ?? []), u]);
  const groups: Group[] = [{ title: null, color: null, mods: [] }];
  const add = (m: Mod | UserMod) => matches(m) && groups[groups.length - 1].mods.push(m);
  const addUnder = (key: string) => {
    for (const u of under.get(key) ?? []) add(u);
    under.delete(key);
  };
  addUnder("");
  for (const r of rows) {
    if (r.kind === "separator") {
      groups.push({ title: r.title, color: r.color, mods: [] });
      addUnder(`${r.title}_separator`);
    } else {
      add(r);
      addUnder(r.name);
    }
  }
  // Under a row the build list no longer shows: the end of the build section, where MO2 has them too.
  for (const key of [...under.keys()]) addUnder(key);
  return groups.filter((g) => g.mods.length > 0);
}
