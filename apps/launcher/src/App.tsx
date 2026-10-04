import { X } from "lucide-react";
import { useEffect } from "react";
import { Header } from "./components/Header";
import { Build } from "./pages/Build";
import { Diagnostics } from "./pages/Diagnostics";
import { Home } from "./pages/Home";
import { Settings } from "./pages/Settings";
import { useApp, type Page } from "./store";

const STATUS_POLL_MS = 3000;

const pages: Record<Page, () => React.JSX.Element | null> = {
  home: Home,
  build: Build,
  diagnostics: Diagnostics,
  settings: Settings,
};

export default function App() {
  const { page, refresh, error, clearError } = useApp();
  const PageView = pages[page];

  useEffect(() => {
    refresh();
    const id = setInterval(refresh, STATUS_POLL_MS);
    return () => clearInterval(id);
  }, [refresh]);

  return (
    <div className="flex h-full flex-col">
      <Header />
      <main className="relative min-h-0 flex-1 overflow-y-auto p-8">
        <PageView />
        {error && (
          <div
            role="alert"
            className="absolute right-6 bottom-6 flex max-w-md items-start gap-3 rounded-lg border border-line border-l-bad border-l-2 bg-raised px-4 py-3 shadow-xl"
          >
            <div className="flex-1 text-[13px]">{error}</div>
            <button onClick={clearError} aria-label="Закрыть" className="text-muted hover:text-fg">
              <X size={15} />
            </button>
          </div>
        )}
      </main>
    </div>
  );
}
