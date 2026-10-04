import { Check, ChevronUp, Download, FolderOpen, GripVertical, Loader2 } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, type DownloadItem, type NexusJob, type RateLimit } from "../api";
import { formatBytes } from "../format";
import { isJobActive, useApp } from "../store";

/** Pixels the pointer moves before a press on a download becomes a drag. */
const DRAG_THRESHOLD = 5;

// The bottom bar, like MO2's status line: downloads on the left, the Nexus allowance on the right.
export function Footer() {
  return (
    <footer className="relative flex h-9 shrink-0 items-center gap-4 border-t border-line/60 bg-surface/60 px-4 text-xs">
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
        className={`flex h-7 items-center gap-2 rounded-full px-3 transition-colors ${open ? "bg-raised text-fg" : "text-muted hover:bg-raised hover:text-fg"}`}
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
        <div className="panel absolute bottom-9 left-0 z-40 flex max-h-[60vh] w-[440px] flex-col overflow-hidden bg-surface shadow-2xl">
          {active.length > 0 && (
            <ul className="divide-y divide-line/60 border-b border-line">
              {active.map((j) => (
                <ActiveJob key={j.id} job={j} onOpen={() => setOpen(false)} />
              ))}
            </ul>
          )}
          <div className="flex h-9 shrink-0 items-center justify-between px-4">
            <span className="label">Последние загрузки</span>
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
          <p className="border-t border-line/60 px-4 py-2 text-[11px] text-faint">
            Перетащите загрузку в список модов, чтобы поставить её в нужное место. Архивы из проводника можно бросать прямо на окно.
          </p>
        </div>
      )}
    </div>
  );
}

function ActiveJob({ job, onOpen }: { job: NexusJob; onOpen: () => void }) {
  const { setFomodJob, setRootJob, setPage } = useApp();
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
    else setPage("nexus");
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
  const { installArchive, setPage } = useApp();
  const label = item.modName || item.fileName;

  // Pointer events, not HTML5 drag and drop: WebView2 drops those while Tauri listens for Explorer files.
  const press = (e: React.PointerEvent) => {
    if (e.button !== 0) return;
    const start = { x: e.clientX, y: e.clientY };
    const move = (ev: PointerEvent) => {
      if (Math.hypot(ev.clientX - start.x, ev.clientY - start.y) < DRAG_THRESHOLD) return;
      cleanup();
      onDragStart();
      setPage("mods");
      useApp.setState({ drag: { file: item.fileName, label, x: ev.clientX, y: ev.clientY, over: null } });
    };
    const cleanup = () => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", cleanup);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", cleanup);
  };

  return (
    <li onPointerDown={press} className="group flex cursor-grab items-center gap-2 px-2 py-1.5 hover:bg-raised/60 active:cursor-grabbing">
      <GripVertical size={13} className="shrink-0 text-faint opacity-0 group-hover:opacity-100" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[13px]">
          {label}
          {item.version && <span className="ml-1.5 font-mono text-[11px] text-muted">{item.version}</span>}
        </div>
        <div className="truncate text-[11px] text-faint">
          {item.fileTitle ? `${item.fileTitle} · ` : ""}
          {formatBytes(item.size)}
        </div>
      </div>
      {item.installed ? (
        <span className="flex shrink-0 items-center gap-1 pr-2 text-ok" title="Установлен">
          <Check size={13} />
        </span>
      ) : (
        <button
          onPointerDown={(e) => e.stopPropagation()}
          onClick={() => void installArchive(item.fileName, null)}
          className="shrink-0 rounded-full px-2.5 py-1 text-muted hover:bg-raised hover:text-fg"
          title="В конец списка ваших модов"
        >
          Установить
        </button>
      )}
    </li>
  );
}

function Limits() {
  const { nexusLimits: l, nexus, setPage } = useApp();
  if (!nexus?.account) {
    return (
      <button onClick={() => setPage("nexus")} className="text-faint hover:text-fg">
        Nexus: не выполнен вход
      </button>
    );
  }
  return (
    <div className="flex items-center gap-3 font-mono text-[11px] text-muted tabular-nums" title={limitsHint(l)}>
      <span className="font-sans text-faint">Nexus API</span>
      <Meter label="в час" left={l?.hourly ?? null} total={l?.hourlyLimit ?? null} />
      <Meter label="в сутки" left={l?.daily ?? null} total={l?.dailyLimit ?? null} />
    </div>
  );
}

function Meter({ label, left, total }: { label: string; left: number | null; total: number | null }) {
  const share = left !== null && total ? left / total : null;
  const tone = share === null ? "text-muted" : share < 0.1 ? "text-bad" : share < 0.3 ? "text-warn" : "text-fg";
  return (
    <span className="flex items-center gap-1.5">
      <span className={tone}>{left === null ? "—" : left.toLocaleString("ru-RU")}</span>
      {total !== null && <span className="text-faint">/ {total.toLocaleString("ru-RU")}</span>}
      <span className="font-sans text-faint">{label}</span>
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
