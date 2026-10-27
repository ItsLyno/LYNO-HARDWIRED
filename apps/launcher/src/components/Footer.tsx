import { Check, ChevronUp, Download, FolderOpen, Loader2, Package, Plus, Rocket, Trash2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, type BuildInfo, type DownloadItem, type NexusJob, type RateLimit } from "../api";
import { downloadLabel, formatBytes, plural } from "../format";
import { Button } from "./Button";
import { Changelog } from "./Changelog";
import { pressToDrag } from "./DragGhost";
import { activeInstance, isJobActive, isRepairOnly, useApp } from "../store";

// A floating rounded badge: the footer itself has no bar, the pills hover over the page.
const PILL = "flex h-8 items-center rounded-full border border-line/60 bg-surface shadow-lg shadow-black/30 backdrop-blur";

// The bottom bar, like MO2's status line: downloads on the left, the Nexus allowance on the right.
export function Footer() {
  return (
    <footer className="relative flex h-12 shrink-0 items-center gap-3 px-4 text-xs">
      <Downloads />
      <Updates />
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
  const failed = nexusJobs.filter((j) => j.state.kind === "failed");
  const asking = active.filter((j) => ["choosing", "choosingRoot"].includes(j.state.kind)).length;
  // The backend forgets every finished job; done and cancelled ones aren't shown anyway.
  const clearFailed = () =>
    run(async () => {
      await api.nexusClearJobs();
      useApp.setState({ nexusJobs: useApp.getState().nexusJobs.filter(isJobActive) });
    });

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
        {failed.length > 0 && (
          <span className="rounded-full bg-bad/15 px-1.5 font-mono text-[10px] text-bad tabular-nums" title="Загрузки с ошибкой">
            {failed.length}
          </span>
        )}
        <ChevronUp size={12} className={`transition-transform ${open ? "" : "rotate-180"}`} />
      </button>
      {open && (
        <div className="panel absolute bottom-11 left-0 z-40 flex max-h-[60vh] w-[420px] flex-col overflow-hidden bg-surface shadow-2xl">
          {active.length > 0 && (
            <ul className="divide-y divide-line/60 border-b border-line">
              {active.map((j) => (
                <ActiveJob key={j.id} job={j} onOpen={() => setOpen(false)} />
              ))}
            </ul>
          )}
          {failed.length > 0 && (
            <div className="border-b border-line">
              <div className="flex h-9 items-center justify-between px-4">
                <span className="label text-bad">Не удалось</span>
                <button onClick={clearFailed} className="text-muted hover:text-fg">
                  Скрыть
                </button>
              </div>
              <ul className="divide-y divide-line/60">
                {failed.map((j) => (
                  <li key={j.id} className="px-4 py-2" title={j.state.kind === "failed" ? j.state.error : undefined}>
                    <div className="truncate text-[13px]">{jobTitle(j)}</div>
                    <div className="line-clamp-2 text-bad">{j.state.kind === "failed" && j.state.error}</div>
                  </li>
                ))}
              </ul>
            </div>
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
            {/* What is still to install comes first; both newest first, as the backend lists them. */}
            {[
              { title: "Не установлены", items: downloads?.filter((d) => !d.installed) ?? [] },
              { title: "Установлены", items: downloads?.filter((d) => d.installed) ?? [] },
            ].map(
              (g) =>
                g.items.length > 0 && (
                  <section key={g.title}>
                    <div className="px-4 pt-2 pb-1 text-[11px] text-faint">{`${g.title} · ${g.items.length}`}</div>
                    <ul>
                      {g.items.map((d) => (
                        <DownloadRow key={d.fileName} item={d} onDragStart={() => setOpen(false)} />
                      ))}
                    </ul>
                  </section>
                ),
            )}
          </div>
        </div>
      )}
    </div>
  );
}

/** Bytes a second of a download from its progress events, smoothed so the number doesn't jump with each one. */
function useRate(done: number | null): number | null {
  const last = useRef<{ t: number; done: number } | null>(null);
  const [rate, setRate] = useState<number | null>(null);
  useEffect(() => {
    if (done === null) {
      last.current = null;
      setRate(null);
      return;
    }
    const now = performance.now();
    const prev = last.current;
    if (prev && done > prev.done && now > prev.t) {
      const r = ((done - prev.done) / (now - prev.t)) * 1000;
      setRate((old) => (old === null ? r : old * 0.7 + r * 0.3));
    }
    if (!prev || done !== prev.done) last.current = { t: now, done };
  }, [done]);
  return rate;
}

function jobTitle(job: NexusJob): string {
  return job.title ?? (job.modId ? `Мод ${job.modId}` : "Загрузка");
}

function ActiveJob({ job, onOpen }: { job: NexusJob; onOpen: () => void }) {
  const { setFomodJob, setRootJob, run } = useApp();
  const s = job.state;
  const title = jobTitle(job);
  const pct = s.kind === "downloading" && s.total > 0 ? (s.done / s.total) * 100 : null;
  const rate = useRate(s.kind === "downloading" ? s.done : null);
  const text: Record<string, string> = {
    queued: "в очереди",
    downloading: pct !== null ? `${Math.floor(pct)}%` : "скачивается",
    retry: s.kind === "retry" ? `переподключение, попытка ${s.attempt}…` : "",
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
    <li className="flex items-center pr-2 hover:bg-raised/60">
      <button onClick={act} className="block min-w-0 flex-1 py-2 pl-4 pr-2 text-left">
        <div className="flex items-center gap-2">
          <span className="min-w-0 flex-1 truncate text-[13px]">{title}</span>
          <span className={`shrink-0 ${s.kind.startsWith("choosing") ? "text-warn" : "text-muted"}`}>{text[s.kind] ?? ""}</span>
        </div>
        {pct !== null && s.kind === "downloading" && (
          <>
            <div className="mt-1.5 h-0.5 rounded-full bg-raised">
              <div className="h-full rounded-full bg-neon transition-[width]" style={{ width: `${pct}%` }} />
            </div>
            <div className="mt-1 flex justify-between font-mono text-[11px] text-faint tabular-nums">
              <span>{`${formatBytes(s.done)} из ${formatBytes(s.total)}`}</span>
              {rate !== null && <span>{`${formatBytes(rate)}/с`}</span>}
            </div>
          </>
        )}
      </button>
      {/* Unpacking into the mod folder doesn't stop halfway; everything before it does. */}
      {s.kind !== "installing" && (
        <button
          onClick={() => run(() => api.nexusCancelJob(job.id))}
          className="flex size-6 shrink-0 items-center justify-center rounded-full text-muted hover:bg-bad/10 hover:text-bad"
          title="Отменить"
          aria-label="Отменить"
        >
          <X size={13} />
        </button>
      )}
    </li>
  );
}

function DownloadRow({ item, onDragStart }: { item: DownloadItem; onDragStart: () => void }) {
  const { installArchive, refreshDownloads, run, setPage } = useApp();
  const [confirm, setConfirm] = useState(false);
  const label = item.modName || item.fileName;
  const when = new Date(item.modified * 1000).toLocaleString("ru-RU", { day: "2-digit", month: "2-digit", hour: "2-digit", minute: "2-digit" });
  const details = [item.fileTitle, formatBytes(item.size), when].filter(Boolean).join(" · ");

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
      title={item.fileName}
      className="group flex cursor-grab items-center gap-1 py-1.5 pl-4 pr-2 hover:bg-raised/60 active:cursor-grabbing"
    >
      <span className="min-w-0 flex-1">
        <span className="block truncate text-[13px]">
          {label}
          {item.version && <span className="ml-1.5 font-mono text-[11px] text-muted">{item.version}</span>}
        </span>
        <span className="block truncate text-[11px] text-faint">{details}</span>
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
      {/* An installed archive installs again too: over its mod, or under another name beside it. */}
      <button
        onPointerDown={(e) => e.stopPropagation()}
        onClick={() => void installArchive(item.fileName, null)}
        className={`${btn} ${item.installed ? "text-ok" : ""}`}
        title={item.installed ? "Установлен. Нажмите, чтобы переустановить или поставить рядом под другим названием" : "Установить в конец списка ваших модов"}
      >
        {item.installed ? <Check size={13} /> : <Plus size={14} />}
      </button>
    </li>
  );
}

/** A new build or launcher: one badge each. Playing doesn't wait for either. The build's opens what it brings
 *  (gigabytes: worth a look first), the launcher's installs at once. */
function Updates() {
  const { status, build, progress, settings, launcherUpdate, run } = useApp();
  const [installing, setInstalling] = useState(false);
  const busy = !!progress || !!status?.updating;
  const isBuild = !!activeInstance(settings)?.build;
  const b = isBuild && build && build.online && !build.upToDate && !settings?.authorMode && !progress ? build : null;
  // The installer closes the launcher and starts the new version, so there is no "done" state.
  const installLauncher = () =>
    run(async () => {
      setInstalling(true);
      try {
        await api.installLauncherUpdate();
      } finally {
        setInstalling(false);
      }
    });
  const pill = `${PILL} gap-2 px-3 text-fg transition-colors hover:bg-raised disabled:cursor-not-allowed disabled:text-faint`;
  return (
    <>
      {b && <BuildUpdate b={b} busy={busy} />}
      {launcherUpdate && (
        <button
          className={pill}
          disabled={installing || busy}
          title={busy ? "Дождитесь окончания обновления сборки" : (launcherUpdate.notes ?? undefined)}
          onClick={installLauncher}
        >
          {installing ? <Loader2 size={13} className="animate-spin text-neon" /> : <Rocket size={13} className="text-neon" />}
          {installing ? "Загрузка лаунчера…" : `Обновить лаунчер до ${launcherUpdate.version}`}
        </button>
      )}
    </>
  );
}

function BuildUpdate({ b, busy }: { b: BuildInfo; busy: boolean }) {
  const { setPage, startUpdate } = useApp();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!open) return;
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
  // Newest first: what came after the installed version is what the update brings.
  const at = b.changelog.findIndex((e) => e.version === b.installedVersion);
  const upcoming = at === -1 ? b.changelog : b.changelog.slice(0, at);
  const label = isRepairOnly(b) ? "Восстановить сборку" : b.installedVersion ? `Сборка ${b.latestVersion}` : "Установить сборку";

  return (
    <div ref={ref} className="relative">
      <button
        onClick={() => setOpen(!open)}
        aria-expanded={open}
        className={`${PILL} gap-2 px-3 transition-colors ${open ? "bg-raised text-fg" : "text-fg hover:bg-raised"}`}
      >
        <Package size={13} className="text-accent" />
        {label}
        <span className="font-mono text-muted tabular-nums">{formatBytes(Math.max(0, b.downloadSize - b.downloaded))}</span>
        <ChevronUp size={12} className={`transition-transform ${open ? "" : "rotate-180"}`} />
      </button>
      {open && (
        <div className="panel absolute bottom-11 left-0 z-40 flex max-h-[60vh] w-[420px] flex-col overflow-hidden bg-surface text-[13px] shadow-2xl">
          <div className="flex items-center gap-3 border-b border-line px-4 py-3">
            <div className="min-w-0 flex-1">
              <div className="font-medium">{label}</div>
              <div className="text-xs text-muted">
                {b.changes} {plural(b.changes, "изменение", "изменения", "изменений")} · {downloadLabel(b)}
              </div>
            </div>
            <Button
              className="h-8 text-[13px]"
              disabled={busy}
              onClick={() => {
                setOpen(false);
                setPage("mods");
                void startUpdate();
              }}
            >
              <Download size={14} />
              {isRepairOnly(b) ? "Восстановить" : b.installedVersion ? "Обновить" : "Установить"}
            </Button>
          </div>
          {upcoming.length > 0 && (
            <div className="min-h-0 overflow-y-auto px-4 py-3">
              <Changelog entries={upcoming} installed={b.installedVersion} />
            </div>
          )}
        </div>
      )}
    </div>
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
