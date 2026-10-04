import { isTauri } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { Minus, Square, X } from "lucide-react";
import type { ReactNode } from "react";
import { useApp, type Page } from "../store";

const tabs: { id: Page; label: string }[] = [
  { id: "home", label: "Главная" },
  { id: "build", label: "Сборка" },
  { id: "diagnostics", label: "Диагностика" },
  { id: "settings", label: "Настройки" },
];

// The window is frameless: this bar is the title bar, navigation and window controls.
export function Header() {
  const { page, setPage } = useApp();
  const win = isTauri() ? getCurrentWindow() : null;

  return (
    <header data-tauri-drag-region className="flex h-12 shrink-0 items-stretch border-b border-line bg-bg pl-5">
      <div data-tauri-drag-region className="mr-8 flex items-center text-[13px] font-semibold tracking-wide">
        LYNO<span className="text-faint">//</span>HARDWIRED
      </div>
      <nav className="flex items-stretch gap-1">
        {tabs.map((t) => (
          <button
            key={t.id}
            onClick={() => setPage(t.id)}
            className={`relative px-3 text-sm transition-colors ${
              page === t.id ? "text-fg" : "text-muted hover:text-fg"
            }`}
          >
            {t.label}
            {page === t.id && <span className="absolute inset-x-3 bottom-0 h-0.5 rounded-full bg-fg" />}
          </button>
        ))}
      </nav>
      <div data-tauri-drag-region className="flex-1" />
      <WinButton label="Свернуть" onClick={() => win?.minimize()}>
        <Minus size={15} />
      </WinButton>
      <WinButton label="Развернуть" onClick={() => win?.toggleMaximize()}>
        <Square size={12} />
      </WinButton>
      <WinButton label="Закрыть" danger onClick={() => win?.close()}>
        <X size={16} />
      </WinButton>
    </header>
  );
}

function WinButton(props: { label: string; danger?: boolean; onClick: () => void; children: ReactNode }) {
  return (
    <button
      aria-label={props.label}
      title={props.label}
      onClick={props.onClick}
      className={`flex w-12 items-center justify-center text-muted transition-colors ${
        props.danger ? "hover:bg-[#c42b1c] hover:text-white" : "hover:bg-raised hover:text-fg"
      }`}
    >
      {props.children}
    </button>
  );
}
