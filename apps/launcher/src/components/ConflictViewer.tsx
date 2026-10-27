import { ArrowRight, Boxes, ChevronDown, ChevronUp, FileStack, RefreshCw, Search, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, OVERWRITE, type ArchiveConflicts, type Conflicts, type ModConflicts } from "../api";
import { plural } from "../format";
import { useApp } from "../store";
import { Chip } from "./Chip";

type Tab = "files" | "archives";
type Filter = "all" | "dead" | "archives";

const FILES = ["файл", "файла", "файлов"] as const;
const RESOURCES = ["ресурс", "ресурса", "ресурсов"] as const;
const PREVIEW = 4;

const FILE_RULE = "Если один и тот же файл есть в нескольких включённых модах, игра получает его из мода ниже в списке (с большим номером). Overwrite важнее всех модов.";
/** What decides the ladder's order, said above it. */
const ORDER_RULE: Record<"files" | Conflicts["archiveOrder"], string> = {
  files: "Моды стоят как в списке. Файл игра берёт у того, кто ниже; Overwrite важнее всех.",
  alphabetical:
    "Архивы стоят в том порядке, в каком их грузит игра: по алфавиту имён, сначала #, цифры и заглавные. Ресурс берётся у того, кто выше. Место мода в списке тут не влияет, только имя архива.",
  mo2: "Моды стоят как в списке. Порядок архивов задаёт MO2: ресурс берётся у того, кто выше.",
  mo2Reversed: "Моды стоят как в списке. Порядок архивов задаёт MO2 (reverse): ресурс берётся у того, кто ниже.",
};

const sum = (xs: { length: number }[]) => xs.reduce((n, x) => n + x.length, 0);

/**
 * What MO2's conflicts tab can't tell: for each file and archive resource several enabled mods have, whose the game gets.
 * A mod's conflicts are a ladder in the order that decides it, so where to move a mod reads off its place.
 */
export function ConflictViewer({
  onClose,
  initial,
  priority,
  own,
  locked,
  onMoved,
}: {
  onClose: () => void;
  initial?: string;
  priority: Map<string, number>;
  /** Whether the launcher may move it: the player's own mods, any in author mode. */
  own: (folder: string) => boolean;
  /** Why nothing can move now. */
  locked: string | null;
  onMoved: () => Promise<void>;
}) {
  const { build } = useApp();
  const [data, setData] = useState<Conflicts | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [moveError, setMoveError] = useState<string | null>(null);
  const [moving, setMoving] = useState(false);
  const [current, setCurrent] = useState<string | null>(initial ?? null);
  const [tab, setTab] = useState<Tab>("files");
  const [filter, setFilter] = useState<Filter>("all");
  const [query, setQuery] = useState("");

  const titles = new Map((build?.mods ?? []).flatMap((m) => (m.kind === "mod" && m.title ? [[m.name, m.title] as const] : [])));
  const title = (folder: string) => (folder === OVERWRITE ? "Overwrite" : (titles.get(folder) ?? folder));

  const refresh = async () => {
    setError(null);
    try {
      const next = await api.modConflicts();
      setData(next);
      setCurrent((c) => (c && next.mods.includes(c) ? c : (next.mods[0] ?? null)));
    } catch (e) {
      setError(String(e));
    }
  };

  useEffect(() => {
    void refresh();
    const esc = (e: KeyboardEvent) => e.key === "Escape" && onClose();
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, []);

  const files = new Map((data?.files ?? []).map((m) => [m.modFolder, m] as const));
  const archives = new Map((data?.archives ?? []).map((m) => [m.modFolder, m] as const));
  const filesDead = (f?: ModConflicts) => !!f && f.files > 0 && f.lost === f.files;
  const isDead = (folder: string) => filesDead(files.get(folder)) || !!archives.get(folder)?.dead.length;
  const mods = data?.mods ?? [];
  const deadCount = mods.filter(isDead).length;
  const q = query.trim().toLowerCase();
  const shown = mods.filter(
    (m) =>
      (filter === "all" || (filter === "dead" ? isDead(m) : archives.has(m))) &&
      (!q || title(m).toLowerCase().includes(q) || m.toLowerCase().includes(q)),
  );

  const pick = (folder: string) => {
    setCurrent(folder);
    // Opening the other side of a pair keeps showing that pair's kind of conflict.
    setTab((t) => (t === "archives" ? (archives.has(folder) ? "archives" : "files") : files.has(folder) ? "files" : "archives"));
  };
  const f = current ? files.get(current) : undefined;
  const a = current ? archives.get(current) : undefined;
  const activeTab: Tab = tab === "files" ? (f || !a ? "files" : "archives") : a || !f ? "archives" : "files";
  const number = (folder: string) => (folder === OVERWRITE ? "OW" : priority.get(folder)?.toString() ?? "—");
  const rank = (folder: string) => (folder === OVERWRITE ? Infinity : (priority.get(folder) ?? Infinity));
  const at = new Map([...priority].map(([folder, n]) => [n, folder] as const));

  const move: Move = {
    allowed: (folder, past) => folder !== OVERWRITE && past !== OVERWRITE && own(folder) && own(past),
    // ponytail: below the mod before `past`, so a separator between them is left under it; drag in the list to place it exactly.
    before: (folder) => at.get(rank(folder) - 1) ?? null,
    locked: locked ?? (moving ? "Перемещаю…" : null),
    run: async (folder, after) => {
      setMoving(true);
      setMoveError(null);
      try {
        await api.moveUserMod(folder, after);
        await onMoved();
        await refresh();
      } catch (e) {
        setMoveError(String(e));
      } finally {
        setMoving(false);
      }
    },
  };
  const names = { title, number, onPick: pick };

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-8" role="dialog" aria-modal="true" onMouseDown={(e) => e.target === e.currentTarget && onClose()}>
      <div className="panel flex h-full max-h-[960px] w-full max-w-[1500px] flex-col overflow-hidden bg-surface shadow-2xl">
        <div className="flex h-16 shrink-0 items-center gap-3 border-b border-line px-5">
          <div className="min-w-0 flex-1">
            <div className="text-sm font-semibold">Конфликты модов</div>
            <div className="truncate text-xs text-muted">Какие включённые моды заменяют файлы и ресурсы друг друга и что в итоге попадает в игру</div>
          </div>
          {data && (
            <div className="flex items-center gap-4 text-xs text-muted">
              <span>
                <span className="font-mono text-fg tabular-nums">{mods.length}</span> {plural(mods.length, "мод", "мода", "модов")} с конфликтами
              </span>
              {deadCount > 0 && (
                <span className="text-bad">
                  <span className="font-mono tabular-nums">{deadCount}</span> не {plural(deadCount, "действует", "действуют", "действуют")} целиком или частично
                </span>
              )}
            </div>
          )}
          <button onClick={() => void refresh()} title="Перечитать с диска" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <RefreshCw size={15} />
          </button>
          <button onClick={onClose} aria-label="Закрыть" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <X size={16} />
          </button>
        </div>

        {error ? (
          <p className="px-5 py-8 text-center text-[13px] text-bad">{error}</p>
        ) : data && mods.length === 0 ? (
          <div className="mx-auto max-w-xl px-5 py-12 text-center text-[13px] text-muted">
            <p className="mb-4 text-fg">Конфликтов нет: ни один файл и ни один ресурс архива не встречается в двух включённых модах.</p>
            <p>{FILE_RULE}</p>
          </div>
        ) : (
          <div className="flex min-h-0 flex-1">
            <div className="flex w-96 shrink-0 flex-col border-r border-line">
              <div className="space-y-2 p-3 pb-2">
                <label className="relative block">
                  <Search size={15} className="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-faint" />
                  <input
                    value={query}
                    onChange={(e) => setQuery(e.target.value)}
                    placeholder="Поиск мода"
                    spellCheck={false}
                    className="h-8 w-full rounded-full bg-raised pr-4 pl-9 text-[13px] outline-none placeholder:text-faint focus:ring-1 focus:ring-neon/50"
                  />
                </label>
                <div className="flex gap-1">
                  <Chip active={filter === "all"} onClick={() => setFilter("all")} count={mods.length}>
                    Все
                  </Chip>
                  <Chip active={filter === "dead"} onClick={() => setFilter("dead")} count={deadCount}>
                    Не действуют
                  </Chip>
                  <Chip active={filter === "archives"} onClick={() => setFilter("archives")} count={archives.size}>
                    Архивы
                  </Chip>
                </div>
              </div>
              <div className="min-h-0 flex-1 overflow-y-auto pb-2">
                {data === null && <p className="px-4 py-2 text-[13px] text-muted">Читаю файлы модов…</p>}
                {data && shown.length === 0 && <p className="px-4 py-2 text-[13px] text-muted">Ничего не найдено</p>}
                {shown.map((m) => {
                  const mf = files.get(m);
                  const ma = archives.get(m);
                  return (
                    <button
                      key={m}
                      onClick={() => setCurrent(m)}
                      className={`flex w-full items-start gap-3 px-4 py-2 text-left transition-colors ${m === current ? "bg-raised" : "hover:bg-raised/50"}`}
                    >
                      <span className="w-7 shrink-0 pt-px text-right font-mono text-[11px] text-faint tabular-nums">{number(m)}</span>
                      <span className="min-w-0 flex-1">
                        <span className={`block truncate text-[13px] ${m === current ? "text-fg" : "text-fg/85"}`}>{title(m)}</span>
                        <span className="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11px]">
                          {mf && <Tally icon={FileStack} won={sum(mf.wins.map((o) => o.files))} lost={mf.lost} />}
                          {ma && <Tally icon={Boxes} won={sum(ma.wins.map((o) => o.resources))} lost={sum(ma.loses.map((o) => o.resources))} />}
                          {isDead(m) && <span className="text-bad">{filesDead(mf) ? "не действует" : "архив не действует"}</span>}
                        </span>
                      </span>
                    </button>
                  );
                })}
              </div>
              <div className="border-t border-line px-4 py-2.5 text-[11px] leading-4 text-faint">
                <span className="inline-flex items-center gap-1">
                  <FileStack size={11} /> файлы
                </span>
                <span className="ml-3 inline-flex items-center gap-1">
                  <Boxes size={11} /> ресурсы архивов
                </span>
                <span className="ml-3 text-ok">+ свои в игре</span>
                <span className="ml-2 text-warn">− закрыты</span>
              </div>
            </div>

            <div className="min-w-0 flex-1 overflow-y-auto">
              {current && data && (
                <div className="px-7 py-6">
                  <div className="flex items-baseline gap-3">
                    <span className="font-mono text-sm text-faint tabular-nums">#{number(current)}</span>
                    <h2 className="min-w-0 truncate text-lg font-semibold">{title(current)}</h2>
                  </div>
                  <div className="mt-4 flex gap-1 border-b border-line">
                    <TabButton active={activeTab === "files"} disabled={!f} onClick={() => setTab("files")} icon={FileStack} count={f ? f.lost + sum(f.wins.map((o) => o.files)) : 0}>
                      Файлы
                    </TabButton>
                    <TabButton
                      active={activeTab === "archives"}
                      disabled={!a}
                      onClick={() => setTab("archives")}
                      icon={Boxes}
                      count={a ? sum(a.wins.map((o) => o.resources)) + sum(a.loses.map((o) => o.resources)) : 0}
                    >
                      Ресурсы архивов
                    </TabButton>
                  </div>
                  {moveError && <p className="mt-4 text-[13px] text-bad">{moveError}</p>}
                  {activeTab === "files" && f ? (
                    <Ladder key={`files|${current}`} {...fileLadder(f, rank)} self={current} rule={ORDER_RULE.files} unit={FILES} move={move} {...names} />
                  ) : a ? (
                    <Ladder
                      key={`archives|${current}`}
                      {...archiveLadder(a, data, rank)}
                      self={current}
                      rule={ORDER_RULE[data.archiveOrder]}
                      unit={RESOURCES}
                      archives
                      // Alphabetical: the archives' names decide, the list doesn't.
                      move={data.archiveOrder === "alphabetical" ? null : move}
                      {...names}
                    />
                  ) : null}
                </div>
              )}
            </div>
          </div>
        )}
      </div>
    </div>
  );
}

function Tally({ icon: Icon, won, lost }: { icon: typeof FileStack; won: number; lost: number }) {
  return (
    <span className="inline-flex items-center gap-1.5 font-mono tabular-nums">
      <Icon size={11} className="text-faint" />
      {won > 0 && <span className="text-ok">+{won}</span>}
      {lost > 0 && <span className="text-warn">−{lost}</span>}
    </span>
  );
}

function TabButton(props: { active: boolean; disabled: boolean; onClick: () => void; icon: typeof FileStack; count: number; children: string }) {
  const Icon = props.icon;
  return (
    <button
      onClick={props.onClick}
      disabled={props.disabled}
      className={`-mb-px inline-flex h-9 items-center gap-2 border-b-2 px-3 text-[13px] transition-colors disabled:cursor-default disabled:text-faint ${
        props.active ? "border-neon text-fg" : "border-transparent text-muted hover:text-fg"
      }`}
    >
      <Icon size={14} />
      {props.children}
      <span className="font-mono text-[11px] text-faint tabular-nums">{props.count}</span>
    </button>
  );
}

type Names = { title: (folder: string) => string; number: (folder: string) => string; onPick: (folder: string) => void };

type Side = { folder: string; archive?: string };
/** A step of the ladder: a mod, or one archive of it. */
type Row = Side & { key: string; note?: { text: string; tone: string } };
/** Rows by key: the one whose copy is hidden and the one whose copy the game gets. */
type Edge = { loser: string; winner: string; items: string[] };
type Steps = { rows: Row[]; edges: Edge[] };

/** Moving a mod in the list from the ladder; `after` as `moveUserMod` takes it. */
type Move = {
  allowed: (folder: string, past: string) => boolean;
  before: (folder: string) => string | null;
  locked: string | null;
  run: (folder: string, after: string) => Promise<void>;
};

function fileLadder(mod: ModConflicts, rank: (folder: string) => number): Steps {
  const inGame = mod.files - mod.lost;
  const self: Row = {
    key: mod.modFolder,
    folder: mod.modFolder,
    note: { text: `в игре ${inGame} из ${mod.files}`, tone: inGame === 0 ? "text-bad" : mod.lost ? "text-warn" : "text-ok" },
  };
  const others = [...mod.wins, ...mod.loses].map((o): Row => ({ key: o.modFolder, folder: o.modFolder }));
  return {
    rows: [self, ...others].sort((x, y) => rank(x.folder) - rank(y.folder) || 0),
    edges: [
      ...mod.loses.map((o) => ({ loser: mod.modFolder, winner: o.modFolder, items: o.files })),
      ...mod.wins.map((o) => ({ loser: o.modFolder, winner: mod.modFolder, items: o.files })),
    ],
  };
}

function archiveLadder(mod: ArchiveConflicts, data: Conflicts, rank: (folder: string) => number): Steps {
  const rows = new Map<string, Row>();
  const row = (folder: string, archive: string) => {
    const key = `${folder}|${archive}`;
    if (!rows.has(key)) rows.set(key, { key, folder, archive });
    return key;
  };
  const edges = [
    ...mod.loses.map((o) => ({ loser: row(mod.modFolder, o.archive), winner: row(o.modFolder, o.other), items: o.resources })),
    ...mod.wins.map((o) => ({ loser: row(o.modFolder, o.other), winner: row(mod.modFolder, o.archive), items: o.resources })),
  ];
  for (const r of rows.values()) {
    if (r.folder !== mod.modFolder) continue;
    const lost = sum(edges.filter((e) => e.loser === r.key).map((e) => e.items));
    r.note = mod.dead.includes(r.archive!)
      ? { text: "не действует", tone: "text-bad" }
      : lost
        ? { text: `перекрыто ${lost}`, tone: "text-warn" }
        : { text: "в игре", tone: "text-ok" };
  }
  const load = (r: Row) => data.archiveLoad.indexOf(r.archive!);
  // Under the MO2 plugin the archives follow the list, so the ladder does too: a move reads the same as in the list.
  const order = data.archiveOrder === "alphabetical" ? (x: Row, y: Row) => load(x) - load(y) : (x: Row, y: Row) => rank(x.folder) - rank(y.folder) || load(x) - load(y);
  return { rows: [...rows.values()].sort(order), edges };
}

const STEP = 42;
const GUTTER = 64;
/** Vertical gap between the ends of arcs meeting at one row. */
const PORT = 6;

/** The mod among the ones it shares files with, top to bottom in the order that decides; arcs lead to whose copy the game gets. */
function Ladder({
  self,
  rows,
  edges,
  rule,
  unit,
  archives,
  move,
  title,
  number,
  onPick,
}: Steps & { self: string; rule: string; unit: readonly [string, string, string]; archives?: boolean; move: Move | null } & Names) {
  const [picked, setPicked] = useState<string | null>(null);
  const index = new Map(rows.map((r, i) => [r.key, i] as const));
  const y = (key: string) => index.get(key)! * STEP + STEP / 2;
  // Arcs meeting at a row end side by side, ordered by where they come from, so they don't cross there.
  const ends = new Map<string, number>();
  for (const r of rows) {
    const touching = edges.filter((e) => e.loser === r.key || e.winner === r.key);
    const from = (e: Edge) => index.get(e.loser === r.key ? e.winner : e.loser)!;
    touching.sort((p, q) => from(p) - from(q)).forEach((e, k) => ends.set(`${r.key}|${e.loser}>${e.winner}`, (k - (touching.length - 1) / 2) * PORT));
  }
  const end = (row: string, e: Edge) => y(row) + ends.get(`${row}|${e.loser}>${e.winner}`)!;
  const max = Math.max(...edges.map((e) => e.items.length), 1);
  const mine = (key: string) => rows[index.get(key)!].folder === self;
  const shared = (r: Row) => edges.filter((e) => e.loser === r.key || e.winner === r.key);
  const takes = (r: Row) => sum(shared(r).filter((e) => e.winner === r.key).map((e) => e.items));
  const gives = (r: Row) => sum(shared(r).filter((e) => e.loser === r.key).map((e) => e.items));
  const side = (key: string): Side => rows[index.get(key)!];

  // The nearest row of another mod that way, and the `after` that puts this one past it.
  const step = (i: number, dir: -1 | 1) => {
    if (!move) return null;
    let j = i + dir;
    while (rows[j] && rows[j].folder === rows[i].folder) j += dir;
    const past = rows[j]?.folder;
    if (!past || !move.allowed(rows[i].folder, past)) return null;
    const after = dir === 1 ? past : move.before(past);
    return after === null ? null : { past, after };
  };

  return (
    <>
      <p className="mt-4 max-w-3xl text-[13px] leading-5 text-muted">{rule}</p>
      <div className="relative mt-4" style={{ height: rows.length * STEP }}>
        <svg aria-hidden="true" width={GUTTER} height={rows.length * STEP} className="absolute top-0 left-0 overflow-visible">
          {edges.map((e) => {
            const [y1, y2] = [end(e.loser, e), end(e.winner, e)];
            const x = GUTTER - 6;
            const bulge = Math.min(10 + (Math.abs(y1 - y2) / STEP) * 10, x - 4);
            const lit = !picked || e.loser === picked || e.winner === picked;
            return (
              <g key={`${e.loser}>${e.winner}`} className={`${mine(e.winner) ? "fill-neon stroke-neon" : "fill-warn stroke-warn"} ${lit ? "opacity-70" : "opacity-15"} transition-opacity`}>
                {/* The line stops short of the head, which ends at the row. */}
                <path
                  d={`M${x},${y1} C${x - bulge},${y1} ${x - bulge},${y2} ${x - 5},${y2}`}
                  fill="none"
                  strokeLinecap="round"
                  strokeWidth={1.25 + 2.25 * Math.sqrt(e.items.length / max)}
                />
                <circle cx={x} cy={y1} r={2} stroke="none" />
                <polygon points={`${x + 1},${y2} ${x - 6},${y2 - 3.5} ${x - 6},${y2 + 3.5}`} stroke="none" />
              </g>
            );
          })}
        </svg>
        <div className="absolute top-0 right-0" style={{ left: GUTTER }}>
          {rows.map((r, i) => {
            const me = r.folder === self;
            const [t, g] = [takes(r), gives(r)];
            const [up, down] = [step(i, -1), step(i, 1)];
            return (
              <div key={r.key} style={{ height: STEP }} className="py-[3px]">
                <div
                  className={`flex h-full items-center gap-1 rounded-lg pr-1 transition-colors ${
                    me ? "bg-neon/10 ring-1 ring-neon/40" : r.key === picked ? "bg-raised ring-1 ring-line" : "bg-raised/40"
                  }`}
                >
                  <button
                    onClick={() => !me && setPicked(r.key === picked ? null : r.key)}
                    disabled={me}
                    className={`flex h-full min-w-0 flex-1 items-center gap-3 rounded-lg px-3 text-left ${me ? "cursor-default" : "hover:bg-raised/70"}`}
                  >
                    <span className="w-8 shrink-0 font-mono text-[11px] text-faint tabular-nums">{archives && !move ? i + 1 : `#${number(r.folder)}`}</span>
                    {archives && <span className={`w-80 shrink-0 truncate font-mono text-[12px] ${me ? "text-fg" : "text-fg/80"}`}>{r.archive}</span>}
                    <span className={`min-w-0 flex-1 truncate text-[13px] ${me ? "font-semibold text-fg" : archives ? "text-muted" : "text-fg/85"}`}>
                      {me ? (archives ? "этот мод" : title(r.folder)) : title(r.folder)}
                    </span>
                    <span className="shrink-0 text-right text-[12px] tabular-nums">
                      {r.note ? (
                        <span className={r.note.tone}>{r.note.text}</span>
                      ) : (
                        <>
                          {t > 0 && <span className="text-warn">забирает {t}</span>}
                          {t > 0 && g > 0 && <span className="text-faint"> · </span>}
                          {g > 0 && <span className="text-neon">уступает {g}</span>}
                        </>
                      )}
                    </span>
                  </button>
                  {move && (
                    <span className="flex w-16 shrink-0 justify-end">
                      <MoveButton dir="up" target={up} locked={move.locked} title={title} onMove={(after) => void move.run(r.folder, after)} />
                      <MoveButton dir="down" target={down} locked={move.locked} title={title} onMove={(after) => void move.run(r.folder, after)} />
                    </span>
                  )}
                </div>
              </div>
            );
          })}
        </div>
      </div>
      {picked ? (
        <div className="mt-6 space-y-2">
          {edges
            .filter((e) => (e.loser === picked || e.winner === picked) && (mine(e.loser) || mine(e.winner)))
            .map((e) => (
              <Duel key={`${e.loser}>${e.winner}`} loser={side(e.loser)} winner={side(e.winner)} self={mine(e.winner) ? "winner" : "loser"} items={e.items} unit={unit} title={title} number={number} onPick={onPick} />
            ))}
          {archives && <p className="text-xs text-faint">Архивы хранят не имена ресурсов, а хэши путей: по такому числу ресурс находится в WolvenKit.</p>}
        </div>
      ) : (
        <p className="mt-5 text-xs text-faint">Нажмите на строку, чтобы увидеть общие {archives ? "ресурсы" : "файлы"}.</p>
      )}
    </>
  );
}

function MoveButton(props: { dir: "up" | "down"; target: { past: string; after: string } | null; locked: string | null; title: (folder: string) => string; onMove: (after: string) => void }) {
  const Icon = props.dir === "up" ? ChevronUp : ChevronDown;
  const hint = props.target && (props.locked ?? `${props.dir === "up" ? "Выше" : "Ниже"} «${props.title(props.target.past)}»`);
  return (
    <button
      onClick={() => props.target && props.onMove(props.target.after)}
      disabled={!props.target || !!props.locked}
      title={hint ?? undefined}
      aria-label={hint ?? (props.dir === "up" ? "Выше" : "Ниже")}
      className={`inline-flex size-7 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg disabled:text-faint/40 disabled:hover:bg-transparent ${props.target ? "" : "invisible"}`}
    >
      <Icon size={15} />
    </button>
  );
}

/** One pair: the mod whose copy is hidden → the one whose copy the game gets, with what they share. */
function Duel(props: { loser: Side; winner: Side; self: "loser" | "winner"; items: string[]; unit: readonly [string, string, string] } & Names) {
  const [all, setAll] = useState(false);
  const items = all ? props.items : props.items.slice(0, PREVIEW);
  const more = props.items.length - PREVIEW;
  const side = (s: Side, won: boolean) => {
    const mine = (props.self === "winner") === won;
    return (
      <button
        onClick={() => !mine && props.onPick(s.folder)}
        disabled={mine}
        title={mine ? undefined : "Открыть конфликты этого мода"}
        className={`min-w-0 flex-1 rounded-md px-3 py-2 text-left transition-colors ${won ? "bg-ok/10" : "bg-raised/60"} ${mine ? "cursor-default" : "hover:ring-1 hover:ring-neon/50"}`}
      >
        <div className="flex items-center gap-2">
          <span className="font-mono text-[11px] text-faint tabular-nums">#{props.number(s.folder)}</span>
          <span className={`truncate text-[13px] ${won ? "text-fg" : "text-muted line-through decoration-faint/60"}`}>{mine ? "этот мод" : props.title(s.folder)}</span>
          {won && <span className="ml-auto shrink-0 rounded-full bg-ok/15 px-2 py-px text-[10px] font-semibold tracking-wide text-ok uppercase">в игре</span>}
        </div>
        {s.archive && <div className="mt-0.5 truncate font-mono text-[11px] text-faint">{s.archive}</div>}
      </button>
    );
  };
  return (
    <div className="rounded-lg border border-line/70 p-2.5">
      <div className="flex items-center gap-2">
        {side(props.loser, false)}
        <ArrowRight size={15} className="shrink-0 text-faint" />
        {side(props.winner, true)}
        <span className="w-24 shrink-0 text-right font-mono text-[11px] text-muted tabular-nums">
          {props.items.length} {plural(props.items.length, ...props.unit)}
        </span>
      </div>
      <ul className="mt-2 space-y-px pl-1 font-mono text-[12px] leading-5 text-fg/75 select-text">
        {items.map((f) => (
          <li key={f} className="truncate" title={f}>
            {f}
          </li>
        ))}
      </ul>
      {more > 0 && (
        <button onClick={() => setAll(!all)} className="mt-1 pl-1 text-xs text-neon hover:underline">
          {all ? "Свернуть" : `Показать ещё ${more}`}
        </button>
      )}
    </div>
  );
}
