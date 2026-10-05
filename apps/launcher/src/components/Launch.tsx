import { ChevronDown, Download, FolderCog, FolderSearch, Loader2, Play, RefreshCw, Wrench } from "lucide-react";
import { useEffect, useRef, useState, type ReactNode } from "react";
import { api, type Executable } from "../api";
import { isRepairOnly, primaryAction, useApp, type PrimaryAction, type Progress } from "../store";
import { Button } from "./Button";

/** The launcher's one job, reachable from every page: start the game (or get it ready), MO2 and its tools. */
export function Launch() {
  const { status, build, progress, settings, run, setPage, startUpdate } = useApp();
  const [launching, setLaunching] = useState(false);
  const action = primaryAction(status, build, !!progress, settings);
  const toolsOff = !status?.mo2Installed || !!progress;

  const onPrimary = async () => {
    switch (action) {
      case "play":
        setLaunching(true);
        await run(api.launchGame);
        // MO2 deploys REDmod before the game window appears; keep the button busy meanwhile.
        setTimeout(() => setLaunching(false), 5000);
        return;
      case "install":
      case "update":
        setPage("mods");
        return startUpdate();
      case "game":
        return setPage("settings");
    }
  };

  return (
    <div className="flex items-center gap-2">
      <ToolsMenu disabled={toolsOff} />
      <Button variant="ghost" disabled={toolsOff} onClick={() => run(api.openMo2)} title="Открыть Mod Organizer 2">
        <FolderCog size={15} />
        MO2
      </Button>
      <Button
        variant="primary"
        className="min-w-36"
        disabled={!["play", "install", "update", "game"].includes(action) || launching}
        onClick={onPrimary}
      >
        {primaryLabel(action, launching, build?.latestVersion, isRepairOnly(build), progress)}
      </Button>
    </div>
  );
}

/** MO2's other executables (REDmod, WolvenKit, what the player added in MO2), started inside its virtual file system. */
function ToolsMenu({ disabled }: { disabled: boolean }) {
  const { run, settings } = useApp();
  const [tools, setTools] = useState<Executable[]>([]);
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);
  useEffect(() => {
    api.executables().then(setTools, () => setTools([]));
  }, [settings?.instanceDir]);
  useEffect(() => {
    if (!open) return;
    const outside = (e: Event) => !ref.current?.contains(e.target as Node) && setOpen(false);
    const esc = (e: KeyboardEvent) => e.key === "Escape" && setOpen(false);
    window.addEventListener("mousedown", outside);
    window.addEventListener("keydown", esc);
    return () => {
      window.removeEventListener("mousedown", outside);
      window.removeEventListener("keydown", esc);
    };
  }, [open]);
  if (tools.length === 0) return null;
  return (
    <div ref={ref} className="relative">
      <Button variant="ghost" disabled={disabled} onClick={() => setOpen(!open)} aria-haspopup="menu" aria-expanded={open}>
        <Wrench size={15} />
        Программы
        <ChevronDown size={14} className={`transition-transform ${open ? "rotate-180" : ""}`} />
      </Button>
      {open && (
        <div role="menu" className="panel absolute top-full right-0 z-50 mt-2 w-72 bg-surface py-1.5 shadow-2xl">
          {tools.map((t) => (
            <button
              key={t.title}
              role="menuitem"
              title={t.binary}
              onClick={() => {
                setOpen(false);
                void run(() => api.launchExecutable(t.title));
              }}
              className="flex h-8 w-full items-center gap-2.5 px-3.5 text-left text-[13px] text-fg outline-none hover:bg-raised focus:bg-raised"
            >
              <Play size={13} className="shrink-0 opacity-70" />
              <span className="truncate">{t.title}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  );
}

function primaryLabel(action: PrimaryAction, launching: boolean, latest: string | undefined, repair: boolean, progress: Progress | null): ReactNode {
  if (launching) return <><Loader2 size={16} className="animate-spin" />Запуск…</>;
  switch (action) {
    case "loading":
      return <Loader2 size={16} className="animate-spin" />;
    case "updating": {
      const b = progress?.bytes;
      return <><Loader2 size={16} className="animate-spin" />{b && b.total > 0 ? `${Math.floor((b.done / b.total) * 100)}%` : "Обновление…"}</>;
    }
    case "running":
      return "Игра запущена";
    case "game":
      return <><FolderSearch size={16} />Указать игру</>;
    case "install":
      return <><Download size={16} />Установить сборку</>;
    case "update":
      return repair ? <><Wrench size={16} />Восстановить</> : <><RefreshCw size={16} />Обновить до {latest}</>;
    case "setup":
    case "play":
      return <><Play size={16} fill="currentColor" />Играть</>;
  }
}
