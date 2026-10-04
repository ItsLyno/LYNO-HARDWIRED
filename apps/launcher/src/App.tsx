import { X } from "lucide-react";
import { useEffect } from "react";
import { api } from "./api";
import { Header } from "./components/Header";
import { LauncherUpdateBar } from "./components/LauncherUpdateBar";
import { Home } from "./pages/Home";
import { Mods } from "./pages/Mods";
import { Nexus } from "./pages/Nexus";
import { Release } from "./pages/Release";
import { Settings } from "./pages/Settings";
import { Updates } from "./pages/Updates";
import { useApp, type Page } from "./store";

const STATUS_POLL_MS = 3000;

const pages: Record<Page, () => React.JSX.Element | null> = {
  home: Home,
  mods: Mods,
  updates: Updates,
  nexus: Nexus,
  release: Release,
  settings: Settings,
};

export default function App() {
  const { page, error, clearError } = useApp();
  const PageView = pages[page];

  useEffect(() => {
    const {
      refreshStatus,
      refreshBuild,
      checkLauncherUpdate,
      onUpdateEvent,
      onUpdateFinished,
      onVerifyEvent,
      onAuthorEvent,
      refreshNexus,
      onNexusJob,
    } = useApp.getState();
    void refreshStatus();
    void refreshBuild();
    void checkLauncherUpdate();
    // The launcher may have been started by an nxm link: its download is already queued.
    void refreshNexus().then(() => {
      if (useApp.getState().nexusJobs.length > 0) useApp.getState().setPage("nexus");
    });
    const poll = setInterval(refreshStatus, STATUS_POLL_MS);
    const unlisten = api.onUpdate(onUpdateEvent, (f) => onUpdateFinished(f.ok, f.error));
    const unlistenVerify = api.onVerifyProgress(onVerifyEvent);
    const unlistenAuthor = api.onAuthorEvent(onAuthorEvent);
    const unlistenNexus = api.onNexus({
      job: onNexusJob,
      check: (p) => useApp.setState({ nexusChecking: p }),
      changed: () => {
        void refreshNexus();
        void refreshStatus();
      },
      link: () => useApp.getState().setPage("nexus"),
    });
    return () => {
      clearInterval(poll);
      void unlisten.then((f) => f());
      void unlistenVerify.then((f) => f());
      void unlistenAuthor.then((f) => f());
      void unlistenNexus.then((f) => f());
    };
  }, []);

  return (
    <div className="flex h-full flex-col">
      <Header />
      <LauncherUpdateBar />
      <main className="relative min-h-0 flex-1 overflow-y-auto px-8 pt-6 pb-8">
        <PageView />
        {error && (
          <div
            role="alert"
            className="fixed right-6 bottom-6 flex max-w-md items-start gap-3 rounded-2xl bg-raised px-4 py-3 shadow-xl"
          >
            <div className="flex-1 text-[13px] text-bad">{error}</div>
            <button onClick={clearError} aria-label="Закрыть" className="text-muted hover:text-fg">
              <X size={15} />
            </button>
          </div>
        )}
      </main>
    </div>
  );
}
