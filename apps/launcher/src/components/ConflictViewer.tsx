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
      <div className="mt-5 grid grid-cols-3 gap-3">
        <Stat label="Файлов в моде" value={mod.files} />
        <Stat label="Попадают в игру" value={inGame} tone={dead ? "text-bad" : "text-ok"} />
        <Stat label="Закрыты другими модами" value={mod.lost} tone={mod.lost ? "text-warn" : undefined} />
      </div>
      <div className="mt-3 flex h-1.5 overflow-hidden rounded-full bg-raised" title={`В игре ${inGame} из ${mod.files}`}>
        <div className="bg-ok/70" style={{ width: `${(inGame / Math.max(mod.files, 1)) * 100}%` }} />
        <div className="bg-warn/70" style={{ width: `${(mod.lost / Math.max(mod.files, 1)) * 100}%` }} />
      </div>
      {dead && (
        <Warning>
          Все файлы мода закрыты модами ниже в списке: в игре он ничего не меняет. Опустите его ниже в списке, если он должен победить, или выключите.
        </Warning>
      )}
      <Section heading="Его файлы закрыты" hint="Игра берёт эти файлы из другого мода" tone="text-warn" empty={mod.loses.length === 0}>
        {mod.loses.map((o) => (
          <Duel key={o.modFolder} loser={self} winner={{ folder: o.modFolder }} self="loser" items={o.files} unit={FILES} title={title} number={number} onPick={onPick} />
        ))}
      </Section>
      <Section heading="Он закрывает чужие файлы" hint="Игра берёт эти файлы из этого мода" tone="text-ok" empty={mod.wins.length === 0}>
        {mod.wins.map((o) => (
          <Duel key={o.modFolder} loser={{ folder: o.modFolder }} winner={self} self="winner" items={o.files} unit={FILES} title={title} number={number} onPick={onPick} />
        ))}
      </Section>
    </>
  );
}

function ArchivesView({ mod, title, number, onPick }: { mod: ArchiveConflicts } & Names) {
  const won = sum(mod.wins.map((o) => o.resources));
  const lost = sum(mod.loses.map((o) => o.resources));
  return (
    <>
      <div className="mt-5 grid grid-cols-3 gap-3">
        <Stat label="Его ресурсы в игре" value={won} tone={won ? "text-ok" : undefined} hint="из тех, что есть и у других модов" />
        <Stat label="Его ресурсы закрыты" value={lost} tone={lost ? "text-warn" : undefined} />
        <Stat label="Архивы без эффекта" value={mod.dead.length} tone={mod.dead.length ? "text-bad" : undefined} />
      </div>
      {mod.dead.length > 0 && (
        <Warning>
          {mod.dead.join(", ")}: все ресурсы {mod.dead.length === 1 ? "архива" : "архивов"} берутся из других модов, в игре {mod.dead.length === 1 ? "он ничего не меняет" : "они ничего не меняют"}.
        </Warning>
      )}
      <Section heading="Его ресурсы закрыты" hint="Игра берёт эти ресурсы из архива другого мода" tone="text-warn" empty={mod.loses.length === 0}>
        {mod.loses.map((o) => (
          <Duel
            key={`${o.archive}|${o.modFolder}|${o.other}`}
            loser={{ folder: mod.modFolder, archive: o.archive }}
            winner={{ folder: o.modFolder, archive: o.other }}
            self="loser"
            items={o.resources}
            unit={RESOURCES}
            title={title}
            number={number}
            onPick={onPick}
          />
        ))}
      </Section>
      <Section heading="Он закрывает чужие ресурсы" hint="Игра берёт эти ресурсы из архива этого мода" tone="text-ok" empty={mod.wins.length === 0}>
        {mod.wins.map((o) => (
          <Duel
            key={`${o.archive}|${o.modFolder}|${o.other}`}
            loser={{ folder: o.modFolder, archive: o.other }}
            winner={{ folder: mod.modFolder, archive: o.archive }}
            self="winner"
            items={o.resources}
            unit={RESOURCES}
            title={title}
            number={number}
            onPick={onPick}
          />
        ))}
      </Section>
      <p className="mt-4 text-xs text-faint">Архивы хранят не имена ресурсов, а хэши путей: по такому числу ресурс находится в WolvenKit.</p>
    </>
  );
}

function Stat({ label, value, tone, hint }: { label: string; value: number; tone?: string; hint?: string }) {
  return (
    <div className="rounded-lg bg-raised/40 px-4 py-3">
      <div className={`font-mono text-2xl tabular-nums ${tone ?? "text-fg"}`}>{value}</div>
      <div className="mt-0.5 text-xs text-muted">{label}</div>
      {hint && <div className="text-[11px] text-faint">{hint}</div>}
    </div>
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

function Section(props: { heading: string; hint: string; tone: string; empty: boolean; children: ReactNode }) {
  if (props.empty) return null;
  return (
    <section className="mt-7">
      <div className="mb-2.5 flex items-baseline gap-2">
        <h3 className={`text-[13px] font-semibold ${props.tone}`}>{props.heading}</h3>
        <span className="text-xs text-faint">{props.hint}</span>
      </div>
      <div className="space-y-2">{props.children}</div>
    </section>
  );
}

type Side = { folder: string; archive?: string };

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
