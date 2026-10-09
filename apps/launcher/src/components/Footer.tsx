import { Check, ChevronUp, Download, FolderOpen, Loader2, Plus, Trash2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, type DownloadItem, type NexusJob, type RateLimit } from "../api";
import { formatBytes } from "../format";
import { pressToDrag } from "./DragGhost";
import { isJobActive, useApp } from "../store";

// A floating rounded badge: the footer itself has no bar, the pills hover over the page.
const PILL = "flex h-8 items-center rounded-full border border-line/60 bg-surface/90 shadow-lg shadow-black/30 backdrop-blur";

// The bottom bar, like MO2's status line: downloads on the left, the Nexus allowance on the right.
export function Footer() {
  return (
    <footer className="relative flex h-12 shrink-0 items-center gap-3 px-4 text-xs">
      <Downloads />
      <div className="flex-1" />
      <Limits />
    </footer>
  );
}

function Downloads() {
  const { nexusJobs, downloads, refreshDownloads, run } = useApp();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  const active = nexusJobs.filter(isJobActive);
  const asking = active.filter((j) => ["choosing", "choosingRoot"].includes(j.state.kind)).length;

  useEffect(() => {
    if (!open) return;
    void refreshDownloads();
    const close = (e: PointerEvent) => {
      if (!ref.current?.contains(e.target as Node)) setOpen(false);
    };
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("pointerdown", close);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("keydown", esc);
    };
  }, [open]);

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className={`${PILL} gap-2 px-3 transition-colors ${open ? "bg-raised text-fg" : "text-muted hover:bg-raised hover:text-fg"}`}
      >
        {active.length > asking ? <Loader2 size={13} className="animate-spin text-neon" /> : <Download size={13} />}
        Загрузки
        {active.length > 0 && (
          <span className={`rounded-full px-1.5 font-mono text-[10px] tabular-nums ${asking ? "bg-warn/15 text-warn" : "bg-neon/15 text-neon"}`}>
            {active.length}
          </span>
        )}
        <ChevronUp size={12} className={`transition-transform ${open ? "" : "rotate-180"}`} />
      </button>
      {open && (
        <div className="panel absolute bottom-11 left-0 z-40 flex max-h-[60vh] w-[380px] flex-col overflow-hidden bg-surface shadow-2xl">
          {active.length > 0 && (
            <ul className="divide-y divide-line/60 border-b border-line">
              {active.map((j) => (
                <ActiveJob key={j.id} job={j} onOpen={() => setOpen(false)} />
              ))}
            </ul>
          )}
          <div className="flex h-9 shrink-0 items-center justify-between px-4">
            <span className="label" title="Перетащите загрузку в список модов, чтобы поставить её в нужное место. Архивы из проводника можно бросать прямо на окно.">
              Последние загрузки
            </span>
            <button
              onClick={() => run(() => api.openFolder("downloads"))}
              className="flex items-center gap-1.5 text-muted hover:text-fg"
              title="Папка загрузок MO2"
            >
              <FolderOpen size={13} />
              Папка
            </button>
          </div>
          <div className="min-h-0 overflow-y-auto pb-1">
            {downloads === null && <p className="px-4 py-3 text-muted">Загрузка…</p>}
            {downloads?.length === 0 && (
              <p className="px-4 pb-3 text-[13px] text-muted">
                Пусто. Скачайте мод кнопкой «Mod Manager Download» на Nexus или перетащите архив из проводника в окно лаунчера.
              </p>
            )}
            <ul>
              {downloads?.map((d) => (
                <DownloadRow key={d.fileName} item={d} onDragStart={() => setOpen(false)} />
              ))}
            </ul>
          </div>
        </div>
      )}
    </div>
  );
}

function ActiveJob({ job, onOpen }: { job: NexusJob; onOpen: () => void }) {
  const { setFomodJob, setRootJob } = useApp();
  const s = job.state;
  const title = job.title ?? (job.modId ? `Мод ${job.modId}` : "Загрузка");
  const pct = s.kind === "downloading" && s.total > 0 ? (s.done / s.total) * 100 : null;
  const text: Record<string, string> = {
    queued: "в очереди",
    downloading: pct !== null ? `${Math.floor(pct)}%` : "скачивается",
    retry: "переподключение…",
    waiting: "ждёт обновления сборки",
    waitingMo2: "поставится, когда закроются MO2 и игра",
    installing: "установка…",
    choosing: "выберите варианты",
    choosingRoot: "выберите папку мода",
  };
  const act = () => {
    onOpen();
    if (s.kind === "choosing") setFomodJob(job.id);
    else if (s.kind === "choosingRoot") setRootJob(job.id);
  };
  return (
    <li>
      <button onClick={act} className="block w-full px-4 py-2 text-left hover:bg-raised/60">
        <div className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-[13px]">{title}</span>
          <span className={`shrink-0 ${s.kind.startsWith("choosing") ? "text-warn" : "text-muted"}`}>{text[s.kind] ?? ""}</span>
        </div>
        {pct !== null && (
          <div className="mt-1.5 h-0.5 rounded-full bg-raised">
            <div className="h-full rounded-full bg-neon transition-[width]" style={{ width: `${pct}%` }} />
          </div>
        )}
      </button>
    </li>
  );
}

function DownloadRow({ item, onDragStart }: { item: DownloadItem; onDragStart: () => void }) {
  const { installArchive, refreshDownloads, run, setPage } = useApp();
  const [confirm, setConfirm] = useState(false);
  const label = item.modName || item.fileName;
  const details = [item.fileTitle, formatBytes(item.size), item.fileName].filter(Boolean).join(" · ");

  const press = (e: React.PointerEvent) =>
    pressToDrag(e, (ev) => {
      onDragStart();
      setPage("mods");
      useApp.setState({ drag: { file: item.fileName, label, x: ev.clientX, y: ev.clientY, over: null } });
    });
  const remove = () => {
    if (!confirm) return setConfirm(true);
    void run(async () => {
      await api.deleteDownload(item.fileName);
      await refreshDownloads();
    });
  };
  const btn = "flex size-6 shrink-0 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg";

  return (
    <li
      onPointerDown={press}
      onPointerLeave={() => setConfirm(false)}
      title={details}
      className="group flex h-8 cursor-grab items-center gap-1 pl-4 pr-2 hover:bg-raised/60 active:cursor-grabbing"
    >
      <span className="min-w-0 flex-1 truncate text-[13px]">
        {label}
        {item.version && <span className="ml-1.5 font-mono text-[11px] text-muted">{item.version}</span>}
      </span>
      {confirm ? (
        <button onPointerDown={(e) => e.stopPropagation()} onClick={remove} className="shrink-0 rounded-full px-2 py-0.5 text-bad hover:bg-bad/10">
          Удалить?
        </button>
      ) : (
        <button onPointerDown={(e) => e.stopPropagation()} onClick={remove} className="flex size-6 shrink-0 items-center justify-center rounded-full text-muted opacity-0 hover:bg-bad/10 hover:text-bad group-hover:opacity-100" title="Удалить архив">
          <Trash2 size={13} />
        </button>
      )}
      {item.installed ? (
        <span className="flex size-6 shrink-0 items-center justify-center text-ok" title="Установлен">
          <Check size={13} />
        </span>
      ) : (
        <button
          onPointerDown={(e) => e.stopPropagation()}
          onClick={() => void installArchive(item.fileName, null)}
          className={btn}
          title="Установить в конец списка ваших модов"
        >
          <Plus size={14} />
        </button>
      )}
    </li>
  );
}

function Limits() {
  const { nexusLimits: l, nexus, setPage } = useApp();
  if (!nexus?.account) {
    return (
      <button onClick={() => setPage("settings")} className={`${PILL} px-3 text-faint hover:text-fg`}>
        Nexus: не выполнен вход
      </button>
    );
  }
  return (
    <div className="flex items-center gap-2" title={limitsHint(l)}>
      <Meter label="в час" left={l?.hourly ?? null} total={l?.hourlyLimit ?? null} />
      <Meter label="в сутки" left={l?.daily ?? null} total={l?.dailyLimit ?? null} />
    </div>
  );
}

// Only what is left: the exact count from Nexus' `x-rl-*-remaining`, tinted as it runs out.
function Meter({ label, left, total }: { label: string; left: number | null; total: number | null }) {
  const share = left !== null && total ? left / total : null;
  const tone = share === null ? "text-muted" : share < 0.1 ? "text-bad" : share < 0.3 ? "text-warn" : "text-fg";
  return (
    <span className={`${PILL} gap-1.5 px-3 text-muted`}>
      Nexus · {label}
      <span className={`font-mono tabular-nums ${tone}`}>{left === null ? "—" : left.toLocaleString("ru-RU")}</span>
    </span>
  );
}

function limitsHint(l: RateLimit | null): string {
  if (!l) return "Остаток станет известен после первого запроса к Nexus";
  // Nexus resets the daily allowance at 00:00 UTC; the hourly one is used once the daily is spent.
  const reset = new Date();
  reset.setUTCHours(24, 0, 0, 0);
  const at = reset.toLocaleTimeString("ru-RU", { hour: "2-digit", minute: "2-digit" });
  return `Запросы к Nexus API с вашего аккаунта. Суточный лимит обновляется в ${at}; когда он исчерпан, действует часовой.`;
}
