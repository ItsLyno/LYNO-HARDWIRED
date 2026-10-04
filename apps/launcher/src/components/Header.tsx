import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X } from "lucide-react";
import type { ReactNode } from "react";
import logo from "../assets/logo.webp";
import { useApp, type Page } from "../store";

const tabs: { id: Page; label: string }[] = [
  { id: "home", label: "Главная" },
  { id: "mods", label: "Моды" },
  { id: "updates", label: "Обновления" },
  { id: "settings", label: "Настройки" },
];

// The window is frameless: this bar is the title bar, navigation and window controls.
export function Header() {
  const { page, setPage, build } = useApp();
  const updateAvailable = !!build && !build.upToDate && build.online;
  const win = isTauri() ? getCurrentWindow() : null;

  return (
    <header data-tauri-drag-region className="flex h-14 shrink-0 items-center gap-6 pr-2 pl-6">
      <div data-tauri-drag-region className="flex items-center">
        {/* pointer-events-none: the drag region only reacts to the element it is set on, not its children. */}
        <img src={logo} alt="LYNO//HARDWIRED" draggable={false} className="pointer-events-none h-5 w-auto" />
      </div>
      <nav className="flex items-center gap-0.5 rounded-full border border-line bg-surface/70 p-1">
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => setPage(t.id)}
            className={`relative h-8 rounded-full px-4 text-[13px] transition-colors ${
              page === t.id ? "bg-raised text-fg shadow-[inset_0_0_0_1px_rgb(0_229_255/0.25)]" : "text-muted hover:text-fg"
            }`}
          >
            {t.label}
            {t.id === "updates" && updateAvailable && (
              <span
                className="absolute top-1.5 right-2 size-1.5 rounded-full bg-accent shadow-[0_0_6px_var(--color-accent)]"
                aria-label="есть обновление"
              />
            )}
          </button>
        ))}
      </nav>
      <div data-tauri-drag-region className="h-full flex-1" />
      <div className="flex items-center gap-1">
        <WinButton label="Свернуть" onClick={() => win?.minimize()}>
          <Minus size={14} />
        </WinButton>
        <WinButton label="Развернуть" onClick={() => win?.toggleMaximize()}>
          <Square size={11} />
        </WinButton>
        <WinButton label="Закрыть" danger onClick={() => win?.close()}>
          <X size={15} />
        </WinButton>
      </div>
    </header>
  );
}

function WinButton(props: { label: string; danger?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      aria-label={props.label}
      title={props.label}
      onClick={props.onClick}
      className={`flex size-9 items-center justify-center rounded-full text-muted transition-colors ${
        props.danger ? "hover:bg-bad hover:text-white" : "hover:bg-raised hover:text-fg"
      }`}
    >
      {props.children}
    </button>
  );
}
