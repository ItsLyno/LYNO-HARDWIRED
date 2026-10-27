import { ArrowRight, Boxes, FileStack, Info, RefreshCw, Search, TriangleAlert, X } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
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
const ARCHIVE_RULE: Record<Conflicts["archiveOrder"], string> = {
  alphabetical:
    "Разные .archive могут заменять один и тот же ресурс (модель, текстуру). Архивы игра грузит по алфавиту имён файлов: сначала #, цифры и заглавные буквы, и ресурс берётся из архива, загруженного первым. Место мода в списке на это не влияет: важно только имя архива.",
  mo2: "Разные .archive могут заменять один и тот же ресурс (модель, текстуру). Порядок архивов задаёт MO2 (enforce_archive_load_order): ресурс берётся из мода выше в списке, то есть наоборот, чем с обычными файлами.",
  mo2Reversed:
    "Разные .archive могут заменять один и тот же ресурс (модель, текстуру). Порядок архивов задаёт MO2 (enforce и reverse_archive_load_order): ресурс берётся из мода ниже в списке, как и с обычными файлами.",
};

const sum = (xs: { length: number }[]) => xs.reduce((n, x) => n + x.length, 0);

/**
 * What MO2's conflicts tab can't tell: for each file and archive resource several enabled mods have, whose the game gets.
 * Every pair is shown as loser → winner, so a mod hidden by a higher one never seems to "overwrite" a third.
 */
export function ConflictViewer({ onClose, initial, priority }: { onClose: () => void; initial?: string; priority: Map<string, number> }) {
  const { build } = useApp();
  const [data, setData] = useState<Conflicts | null>(null);
  const [error, setError] = useState<string | null>(null);
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
                  {activeTab === "files" && f ? (
                    <FilesView key={current} mod={f} title={title} number={number} onPick={pick} />
                  ) : a ? (
                    <ArchivesView key={current} mod={a} title={title} number={number} onPick={pick} />
                  ) : null}
                  <div className="mt-8 flex gap-2.5 rounded-lg bg-raised/40 px-4 py-3 text-xs leading-5 text-muted">
                    <Info size={14} className="mt-0.5 shrink-0 text-faint" />
                    <span>{activeTab === "files" ? FILE_RULE : ARCHIVE_RULE[data.archiveOrder]}</span>
                  </div>
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

function FilesView({ mod, title, number, onPick }: { mod: ModConflicts } & Names) {
  const inGame = mod.files - mod.lost;
  const dead = mod.files > 0 && mod.lost === mod.files;
  const self = { folder: mod.modFolder };
  return (
    <>
      <p className="mt-4 text-xs text-muted">
        В игре <span className={dead ? "text-bad" : "text-ok"}>{inGame}</span> из {mod.files} {plural(mod.files, ...FILES)} мода, закрыто другими модами:{" "}
        <span className={mod.lost ? "text-warn" : undefined}>{mod.lost}</span>
      </p>
      <div className="mt-2 flex h-1.5 overflow-hidden rounded-full bg-raised" title={`В игре ${inGame} из ${mod.files}`}>
        <div className="bg-ok/70" style={{ width: `${(inGame / Math.max(mod.files, 1)) * 100}%` }} />
        <div className="bg-warn/70" style={{ width: `${(mod.lost / Math.max(mod.files, 1)) * 100}%` }} />
      </div>
      {dead && (
        <Warning>
          Все файлы мода закрыты модами ниже в списке: в игре он ничего не меняет. Опустите его ниже в списке, если он должен победить, или выключите.
        </Warning>
      )}
      <Graph
        key={mod.modFolder}
        self={mod.modFolder}
        beats={mod.wins.map((o) => ({ folder: o.modFolder, pairs: [{ loser: { folder: o.modFolder }, winner: self, items: o.files }] }))}
        beaten={mod.loses.map((o) => ({ folder: o.modFolder, pairs: [{ loser: self, winner: { folder: o.modFolder }, items: o.files }] }))}
        unit={FILES}
        title={title}
        number={number}
        onPick={onPick}
      />
    </>
  );
}

function ArchivesView({ mod, title, number, onPick }: { mod: ArchiveConflicts } & Names) {
  const won = sum(mod.wins.map((o) => o.resources));
  const lost = sum(mod.loses.map((o) => o.resources));
  return (
    <>
      <p className="mt-4 text-xs text-muted">
        Общих с другими модами ресурсов: в игре из этого мода <span className={won ? "text-ok" : undefined}>{won}</span>, закрыто{" "}
        <span className={lost ? "text-warn" : undefined}>{lost}</span>
      </p>
      {mod.dead.length > 0 && (
        <Warning>
          {mod.dead.join(", ")}: все ресурсы {mod.dead.length === 1 ? "архива" : "архивов"} берутся из других модов, в игре {mod.dead.length === 1 ? "он ничего не меняет" : "они ничего не меняют"}.
        </Warning>
      )}
      <Graph
        key={mod.modFolder}
        self={mod.modFolder}
        beats={byMod(mod.wins.map((o) => ({ folder: o.modFolder, loser: { folder: o.modFolder, archive: o.other }, winner: { folder: mod.modFolder, archive: o.archive }, items: o.resources })))}
        beaten={byMod(mod.loses.map((o) => ({ folder: o.modFolder, loser: { folder: mod.modFolder, archive: o.archive }, winner: { folder: o.modFolder, archive: o.other }, items: o.resources })))}
        unit={RESOURCES}
        title={title}
        number={number}
        onPick={onPick}
      />
      <p className="mt-4 text-xs text-faint">Архивы хранят не имена ресурсов, а хэши путей: по такому числу ресурс находится в WolvenKit.</p>
    </>
  );
}

function Warning({ children }: { children: ReactNode }) {
  return (
    <div className="mt-4 flex items-start gap-2 rounded-lg bg-bad/10 px-3 py-2.5 text-[13px] text-bad">
      <TriangleAlert size={15} className="mt-0.5 shrink-0" />
      <span>{children}</span>
    </div>
  );
}

type Side = { folder: string; archive?: string };
type Pair = { loser: Side; winner: Side; items: string[] };
/** Another mod and everything this one shares with it on one side of the graph. */
type Link = { folder: string; pairs: Pair[] };

const count = (l: Link) => sum(l.pairs.map((p) => p.items));

/** Archive overlaps come per pair of archives; the graph draws one node per mod. */
function byMod(pairs: (Pair & { folder: string })[]): Link[] {
  const out = new Map<string, Pair[]>();
  for (const { folder, ...p } of pairs) out.set(folder, [...(out.get(folder) ?? []), p]);
  return [...out].map(([folder, pairs]) => ({ folder, pairs }));
}

const ROW = 52;
const NODE = 40;
// Column edges in % of the width: losers | this mod | winners.
const LEFT = 30;
const MID = [38, 62];
const RIGHT = 70;

/**
 * The mod among the ones it shares files with, laid out as the game decides: copies flow to the right,
 * the rightmost is what the game gets. Nodes select a link; its files are listed under the graph.
 */
function Graph({ self, beats, beaten, unit, title, number, onPick }: { self: string; beats: Link[]; beaten: Link[]; unit: readonly [string, string, string] } & Names) {
  const [picked, setPicked] = useState<{ side: "beats" | "beaten"; folder: string } | null>(() =>
    beaten[0] ? { side: "beaten", folder: beaten[0].folder } : beats[0] ? { side: "beats", folder: beats[0].folder } : null,
  );
  const height = Math.max(beats.length, beaten.length, 1) * ROW;
  const ys = (n: number) => Array.from({ length: n }, (_, i) => (height - n * ROW) / 2 + i * ROW + ROW / 2);
  const max = Math.max(...[...beats, ...beaten].map(count), 1);
  const width = (l: Link) => 1.5 + 5 * Math.sqrt(count(l) / max);
  const isPicked = (side: "beats" | "beaten", l: Link) => picked?.side === side && picked.folder === l.folder;
  const selected = picked && (picked.side === "beats" ? beats : beaten).find((l) => l.folder === picked.folder);
  const mid = height / 2;
  const curve = (x1: number, y1: number, x2: number, y2: number) => `M${x1},${y1} C${(x1 + x2) / 2},${y1} ${(x1 + x2) / 2},${y2} ${x2},${y2}`;

  const node = (side: "beats" | "beaten", l: Link, y: number) => {
    const on = isPicked(side, l);
    return (
      <button
        key={`${side}|${l.folder}`}
        onClick={() => setPicked({ side, folder: l.folder })}
        style={{ top: y - NODE / 2, height: NODE, width: `${LEFT}%`, ...(side === "beats" ? { left: 0 } : { left: `${RIGHT}%` }) }}
        className={`absolute flex items-center gap-2 rounded-md px-3 text-left transition-colors ${side === "beaten" ? "bg-ok/10" : "bg-raised/60"} ${
          on ? "ring-1 ring-neon" : "hover:ring-1 hover:ring-neon/40"
        }`}
      >
        <span className="shrink-0 font-mono text-[11px] text-faint tabular-nums">#{number(l.folder)}</span>
        <span className={`min-w-0 flex-1 truncate text-[13px] ${side === "beaten" ? "text-fg" : "text-muted"}`}>{title(l.folder)}</span>
        <span className={`shrink-0 font-mono text-[11px] tabular-nums ${side === "beaten" ? "text-warn" : "text-ok"}`}>{count(l)}</span>
      </button>
    );
  };

  return (
    <>
      <div className="mt-6 mb-2 flex text-[11px] font-semibold tracking-wide uppercase">
        <span className="text-ok" style={{ width: `${LEFT}%` }}>
          Он закрывает
        </span>
        <span className="text-center text-faint" style={{ marginLeft: `${MID[0] - LEFT}%`, width: `${MID[1] - MID[0]}%` }}>
          →&nbsp;в игру попадает правый&nbsp;→
        </span>
        <span className="ml-auto text-right text-warn" style={{ width: `${100 - RIGHT}%` }}>
          Закрывают его
        </span>
      </div>
      <div className="relative" style={{ height }}>
        <svg className="absolute inset-0 h-full w-full overflow-visible" viewBox={`0 0 100 ${height}`} preserveAspectRatio="none" aria-hidden>
          {beats.map((l, i) => (
            <path key={l.folder} d={curve(LEFT, ys(beats.length)[i], MID[0], mid)} fill="none" vectorEffect="non-scaling-stroke" strokeWidth={width(l)}
              className={isPicked("beats", l) ? "stroke-neon" : "stroke-ok/35"} />
          ))}
          {beaten.map((l, i) => (
            <path key={l.folder} d={curve(MID[1], mid, RIGHT, ys(beaten.length)[i])} fill="none" vectorEffect="non-scaling-stroke" strokeWidth={width(l)}
              className={isPicked("beaten", l) ? "stroke-neon" : "stroke-warn/35"} />
          ))}
        </svg>
        {beats.map((l, i) => node("beats", l, ys(beats.length)[i]))}
        <div
          style={{ top: mid - NODE / 2 - 6, height: NODE + 12, left: `${MID[0]}%`, width: `${MID[1] - MID[0]}%` }}
          className="absolute flex items-center gap-2 rounded-lg bg-raised px-3 ring-1 ring-line"
        >
          <span className="shrink-0 font-mono text-[11px] text-faint tabular-nums">#{number(self)}</span>
          <span className="min-w-0 truncate text-[13px] font-semibold">{title(self)}</span>
        </div>
        {beaten.map((l, i) => node("beaten", l, ys(beaten.length)[i]))}
      </div>
      {selected && (
        <div className="mt-6 space-y-2">
          {selected.pairs.map((p) => (
            <Duel
              key={`${p.loser.archive}|${p.winner.archive}`}
              loser={p.loser}
              winner={p.winner}
              self={p.winner.folder === self ? "winner" : "loser"}
              items={p.items}
              unit={unit}
              title={title}
              number={number}
              onPick={onPick}
            />
          ))}
        </div>
      )}
    </>
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
