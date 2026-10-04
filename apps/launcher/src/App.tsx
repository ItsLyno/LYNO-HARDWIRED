import { X } from "lucide-react";
import { useEffect } from "react";
import { api } from "./api";
import { DragGhost } from "./components/DragGhost";
import { FomodWizard } from "./components/FomodWizard";
import { Footer } from "./components/Footer";
import { RootPicker } from "./components/RootPicker";
import { Header } from "./components/Header";
import { LauncherUpdateBar } from "./components/LauncherUpdateBar";
import { Home } from "./pages/Home";
import { Mods } from "./pages/Mods";
import { Nexus } from "./pages/Nexus";
import { Release } from "./pages/Release";
import { Settings } from "./pages/Settings";
import { Updates } from "./pages/Updates";
import { dropSpotAt, useApp, type Page } from "./store";

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
  const { page, error, clearError, fomodJob, setFomodJob, rootJob, setRootJob } = useApp();
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
      refreshDownloads,
      refreshUserMods,
    } = useApp.getState();
    void refreshStatus();
    void refreshBuild();
    void checkLauncherUpdate();
    // The launcher may have been started by an nxm link: its download is already queued.
    void refreshNexus().then(() => {
      if (useApp.getState().nexusJobs.length > 0) useApp.getState().setPage("nexus");
    });
    void refreshDownloads();
    void refreshUserMods();
    void api.nexusLimits().then((l) => l && useApp.setState({ nexusLimits: l }));
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
        void refreshUserMods();
      },
      link: () => useApp.getState().setPage("nexus"),
      limits: (l) => useApp.setState({ nexusLimits: l }),
    });
    // Archives dropped from Explorer, like on MO2's window: where they land in the list is where they go.
    const unlistenDrop = api.onFileDrop({
      over: (at) => useApp.setState({ fileOver: dropSpotAt(at.x, at.y) ?? { after: null, outside: true } }),
      leave: () => useApp.setState({ fileOver: null }),
      drop: (paths, at) => {
        const spot = dropSpotAt(at.x, at.y);
        useApp.setState({ fileOver: null });
        for (const p of paths) void useApp.getState().installArchive(p, spot?.after ?? null);
        if (paths.length > 0) useApp.getState().setPage("mods");
      },
    });
    return () => {
      clearInterval(poll);
      void unlisten.then((f) => f());
      void unlistenVerify.then((f) => f());
      void unlistenAuthor.then((f) => f());
      void unlistenNexus.then((f) => f());
      void unlistenDrop.then((f) => f());
    };
  }, []);

  return (
    <div className="flex h-full flex-col">
      <Header />
      <LauncherUpdateBar />
      <main className="relative min-h-0 flex-1 overflow-y-auto px-8 pt-6 pb-8">
        <PageView />
        {fomodJob !== null && <FomodWizard key={fomodJob} jobId={fomodJob} onClose={() => setFomodJob(null)} />}
        {rootJob !== null && <RootPicker key={rootJob} jobId={rootJob} onClose={() => setRootJob(null)} />}
        {error && (
          <div
            role="alert"
            className="fixed right-6 bottom-16 flex max-w-md items-start gap-3 rounded-2xl bg-raised px-4 py-3 shadow-xl"
          >
            <div className="flex-1 text-[13px] text-bad">{error}</div>
            <button onClick={clearError} aria-label="Закрыть" className="text-muted hover:text-fg">
              <X size={15} />
            </button>
          </div>
        )}
      </main>
      <Footer />
      <DragGhost />
    </div>
  );
}
