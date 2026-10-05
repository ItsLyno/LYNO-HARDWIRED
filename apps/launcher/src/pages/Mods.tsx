import { ChevronRight, ExternalLink, FolderOpen, Lock, MoreHorizontal, Pencil, RefreshCw, Search, Trash2, X, type LucideIcon } from "lucide-react";
import { useEffect, useLayoutEffect, useRef, useState, type CSSProperties, type MouseEvent, type ReactNode } from "react";
import { api, type ModRow, type UserRow } from "../api";
import { Button } from "../components/Button";
import { PageTitle } from "../components/PageTitle";
import { plural } from "../format";
import { activeInstance, useApp } from "../store";

type Mod = Extract<ModRow, { kind: "mod" }>;
type Group = { title: string | null; color: string | null; mods: Mod[] };
type Filter = "all" | "off" | "changes";
/** A row the context menu acts on. `id`: manifest id of a build mod. */
type Target = { folder: string; label: string; id: string | null; nexusUrl: string | null; installed: boolean };
type Menu = { target: Target; x: number; y: number };
type Dialog = { kind: "delete" | "reinstall" | "rename"; target: Target };

/** MO2's folder of the separator above the player's own mods (`plan::USER_SEPARATOR`). */
const USER_SEPARATOR = "LYNO USER MODS_separator";

export function Mods() {
  const { build, buildError, status, progress, setModEnabled, settings, nexusUpdates, userMods, refreshUserMods, drag, fileOver } =
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
  const [menu, setMenu] = useState<Menu | null>(null);
  const [dialog, setDialog] = useState<Dialog | null>(null);
  const openMenu = (e: MouseEvent, target: Target) => {
    e.preventDefault();
    setMenu({ target, x: e.clientX, y: e.clientY });
  };
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
  const offCount = all.filter((m) => !m.enabled).length;
  const changeCount = all.filter(isChange).length;
  // MO2 rewrites modlist.txt on exit, so switching mods while it runs would be lost.
  const locked = !!progress || !!status?.gameRunning || !!status?.mo2Running;
  const lockReason = progress
    ? "Дождитесь окончания обновления"
    : "Закройте игру и Mod Organizer 2, чтобы включать и выключать моды";

  // The player's own MO2: the whole list is their section.
  const isBuild = !!activeInstance(settings)?.build;
  if (!build && isBuild) {
    return <p className="text-muted">{buildError ?? "Загрузка списка модов…"}</p>;
  }
  const last = build?.lastUpdate ?? null;
  // Where an archive being dragged or dropped from Explorer would land. A player's mod never goes among the
  // build's: over those it lands at the top of their own section, which is what the core does with it too.
  const spot = drag?.over ?? (fileOver && !fileOver.outside ? fileOver : null);
  const dropping = !!drag || (!!fileOver && !fileOver.outside);
  const buildNames = new Set((build?.mods ?? []).map((r) => (r.kind === "mod" ? r.name : `${r.title}_separator`)));
  const lineAfter = spot && (spot.after === null ? "end" : !settings?.authorMode && buildNames.has(spot.after) ? USER_SEPARATOR : spot.after);
  const userRows = (userMods ?? []).filter((r) => r.kind === "separator" || filter === "all" || (filter === "off" && !r.enabled));
  const q = query.trim().toLowerCase();
  const shownUser = userRows.filter((r) => !q || r.kind === "separator" || r.name.toLowerCase().includes(q));
  const userModCount = (userMods ?? []).filter((r) => r.kind === "mod").length;
  const endKey = shownUser.length > 0 ? rowKey(shownUser[shownUser.length - 1]) : USER_SEPARATOR;
  const line = (key: string) => (lineAfter === key || (lineAfter === "end" && key === endKey) ? "shadow-[inset_0_-2px_0_0_var(--color-neon)]" : "");

  return (
    <div className="mx-auto flex h-full max-w-5xl flex-col">
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
            <col className="w-32" />
            <col className="w-24" />
          </colgroup>
          <thead className="sticky top-0 z-10 bg-surface">
            <tr className="label h-10 border-b border-line">
              <th className="pl-5 font-medium">Название</th>
              <th className="font-medium">Версия</th>
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
                <tr
                  key={m.id}
                  data-drop-after={m.name}
                  onClick={(e) => openMenu(e, buildTarget(m))}
                  onContextMenu={(e) => openMenu(e, buildTarget(m))}
                  className={`h-12 cursor-pointer border-b border-line/60 last:border-b-0 hover:bg-raised/50 ${menu?.target.folder === m.name ? "bg-raised/50" : ""} ${line(m.name)}`}
                >
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
                    {!m.installed && build?.installedVersion && <Badge tone="warn">новый</Badge>}
                    {nexusUpdate.has(m.name) && (
                      <Badge tone="ok" title={`На Nexus: ${nexusUpdate.get(m.name) ?? "новая версия"}. Обновить — на вкладке «Nexus»`}>
                        обновление на Nexus
                      </Badge>
                    )}
                  </td>
                  <td className="truncate font-mono text-xs text-muted tabular-nums">{m.version ?? "—"}</td>
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
          {(shownUser.length > 0 || dropping) && filter !== "changes" && (
            <tbody>
              <SeparatorRow
                title={isBuild ? "Мои моды" : "Моды"}
                color={null}
                count={shownUser.filter((r) => r.kind === "mod").length}
                open={!!q || dropping || !collapsed.has(USER_SEPARATOR)}
                onToggle={() => toggleGroup(USER_SEPARATOR)}
                dropKey={USER_SEPARATOR}
                lineClass={line(USER_SEPARATOR)}
              />
              {(q || dropping || !collapsed.has(USER_SEPARATOR)) &&
                shownUser.map((r) => (
                  <UserModRow
                    key={rowKey(r)}
                    row={r}
                    locked={locked}
                    lockReason={lockReason}
                    lineClass={line(rowKey(r))}
                    active={menu?.target.folder === rowKey(r)}
                    onMenu={(x, y) => r.kind === "mod" && setMenu({ target: userTarget(r), x, y })}
                  />
                ))}
              {dropping && shownUser.length === 0 && (
                <tr data-drop-after={USER_SEPARATOR}>
                  <td colSpan={3} className="px-5 py-4 text-center text-[13px] text-neon">
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

      {menu && <ModMenu menu={menu} locked={locked} lockReason={lockReason} onDialog={setDialog} onClose={() => setMenu(null)} />}
      {dialog && <ModDialog dialog={dialog} onClose={() => setDialog(null)} />}
    </div>
  );
}

function buildTarget(m: Mod): Target {
  return { folder: m.name, label: m.title ?? m.name, id: m.id, nexusUrl: m.nexusUrl, installed: m.installed };
}

function userTarget(r: Extract<UserRow, { kind: "mod" }>): Target {
  return { folder: r.name, label: r.name, id: null, nexusUrl: r.nexusUrl, installed: true };
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
function ModMenu(props: { menu: Menu; locked: boolean; lockReason: string; onDialog: (d: Dialog) => void; onClose: () => void }) {
  const { run, settings } = useApp();
  const ref = useRef<HTMLDivElement>(null);
  const [pos, setPos] = useState({ left: props.menu.x, top: props.menu.y });
  const t = props.menu.target;
  const author = !!settings?.authorMode;
  const own = !t.id || author;
  const dialog = (kind: Dialog["kind"]) => () => props.onDialog({ kind, target: t });
  // A player's build mod comes again with the build (repair), which needs MO2 closed; an archive install waits for it.
  const repair = !!t.id && !author;
  const items: (Item | null)[] = [
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
    {
      label: "Удалить",
      icon: Trash2,
      danger: true,
      disabled: !own || props.locked,
      title: !own ? "Мод сборки можно только выключить" : props.locked ? props.lockReason : undefined,
      onClick: dialog("delete"),
    },
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
      <div className="truncate px-3.5 pt-1 pb-2 text-[12px] font-semibold text-muted">{t.label}</div>
      {items.map((it, i) =>
        it === null ? (
          <div key={i} className="my-1.5 border-t border-line" />
        ) : (
          <button
            key={it.label}
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
            {it.label}
          </button>
        ),
      )}
    </div>
  );
}

function ModDialog({ dialog, onClose }: { dialog: Dialog; onClose: () => void }) {
  const { run, settings, refreshUserMods, refreshBuild, startRepair } = useApp();
  const t = dialog.target;
  const [name, setName] = useState(t.folder);
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
  const rename = () => {
    if (newName && newName !== t.folder) act(() => api.renameMod(t.folder, newName));
  };

  const view: Record<Dialog["kind"], { title: string; body: ReactNode; confirm: string; danger?: boolean; ok: () => void }> = {
    delete: {
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
    rename: {
      title: "Переименовать мод",
      body: (
        <input
          autoFocus
          value={name}
          onChange={(e) => setName(e.target.value)}
          onFocus={(e) => e.target.select()}
          onKeyDown={(e) => e.key === "Enter" && rename()}
          spellCheck={false}
          className="h-9 w-full rounded-lg bg-raised px-3 text-sm text-fg outline-none focus:ring-1 focus:ring-neon/50"
        />
      ),
      confirm: "Переименовать",
      ok: rename,
    },
  };
  const v = view[dialog.kind];

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
            autoFocus={dialog.kind !== "rename"}
            disabled={dialog.kind === "rename" && (!newName || newName === t.folder)}
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
function UserModRow(props: { row: UserRow; locked: boolean; lockReason: string; lineClass: string; active: boolean; onMenu: (x: number, y: number) => void }) {
  const { run, refreshUserMods } = useApp();
  const r = props.row;
  if (r.kind === "separator") {
    return (
      <tr data-drop-after={rowKey(r)} className={`h-9 border-b border-line/60 ${props.lineClass}`}>
        <td colSpan={3} className="pl-5 text-[12px] font-semibold text-muted">
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
    <tr
      data-drop-after={r.name}
      onClick={(e) => props.onMenu(e.clientX, e.clientY)}
      onContextMenu={(e) => {
        e.preventDefault();
        props.onMenu(e.clientX, e.clientY);
      }}
      className={`h-12 cursor-pointer border-b border-line/60 last:border-b-0 hover:bg-raised/50 ${props.active ? "bg-raised/50" : ""} ${props.lineClass}`}
    >
      <td className="truncate pl-5">
        <span className={r.enabled ? "" : "text-faint"}>{r.name}</span>
        {!r.enabled && <Badge>выключен</Badge>}
      </td>
      <td className="truncate font-mono text-xs text-muted tabular-nums">{r.version ?? "—"}</td>
      <td className="pr-5">
        <div className="flex items-center justify-end gap-1" onClick={(e) => e.stopPropagation()}>
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
      <td colSpan={3} className="sticky top-10 z-[5] p-0">
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
    keep(m) && (!q || [m.name, m.title].some((s) => s?.toLowerCase().includes(q)));
  const groups: Group[] = [{ title: null, color: null, mods: [] }];
  for (const r of rows) {
    if (r.kind === "separator") groups.push({ title: r.title, color: r.color, mods: [] });
    else if (matches(r)) groups[groups.length - 1].mods.push(r);
  }
  return groups.filter((g) => g.mods.length > 0);
}
