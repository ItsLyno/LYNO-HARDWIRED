import { X } from "lucide-react";
import { useEffect } from "react";
import { api } from "./api";
import { DragGhost } from "./components/DragGhost";
import { InstanceSetup } from "./components/InstanceSetup";
import { PageTitle } from "./components/PageTitle";
import { FomodWizard } from "./components/FomodWizard";
import { Footer } from "./components/Footer";
import { ReplaceConfirm } from "./components/ReplaceConfirm";
import { RootPicker } from "./components/RootPicker";
import { Header } from "./components/Header";
import { Mods } from "./pages/Mods";
import { Release } from "./pages/Release";
import { Settings } from "./pages/Settings";
import { checkDue, dropSpotAt, primaryAction, useApp, type Page } from "./store";

const STATUS_POLL_MS = 3000;
// How often the launcher asks itself whether a timed update check is due; the interval itself is a setting.
const CHECK_TICK_MS = 10 * 60_000;

function checkIfDue() {
  const { settings, lastCheck, checkUpdates } = useApp.getState();
  if (settings && checkDue(settings.updateCheckHours, lastCheck)) void checkUpdates();
}

const pages: Record<Page, () => React.JSX.Element | null> = {
  mods: Mods,
  release: Release,
  settings: Settings,
};

export default function App() {
  const { page, error, clearError, notice, fomodJob, setFomodJob, rootJob, setRootJob, status, build, progress, settings } = useApp();
  // No MO2 yet: there is no mod list to show, only where to get one.
  const setup = page === "mods" && primaryAction(status, build, !!progress, settings) === "setup";
  const PageView = setup ? FirstRun : pages[page];

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
    void refreshBuild();
    void checkLauncherUpdate();
    // The launcher may have been started by an nxm link: its download is already queued and shows in the footer.
    void refreshNexus();
    // The interval is a setting: due or not is known once settings are loaded.
    void refreshStatus().then(checkIfDue);
    void refreshDownloads();
    void refreshUserMods();
    void api.nexusLimits().then((l) => l && useApp.setState({ nexusLimits: l }));
    const poll = setInterval(refreshStatus, STATUS_POLL_MS);
    const checks = setInterval(checkIfDue, CHECK_TICK_MS);
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
        // The author may have updated a build mod: the mods table shows installed versions.
        void refreshBuild();
      },
      link: () => {},
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
      clearInterval(checks);
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
      <main className="relative min-h-0 flex-1 overflow-y-auto px-8 pt-6 pb-8">
        <PageView />
        {fomodJob !== null && <FomodWizard key={fomodJob} jobId={fomodJob} onClose={() => setFomodJob(null)} />}
        {rootJob !== null && <RootPicker key={rootJob} jobId={rootJob} onClose={() => setRootJob(null)} />}
        <ReplaceConfirm />
        {(error || notice) && (
          <div
            role="alert"
            className="fixed right-6 bottom-16 flex max-w-md items-start gap-3 rounded-2xl bg-raised px-4 py-3 shadow-xl"
          >
            <div className={`flex-1 text-[13px] ${error ? "text-bad" : "text-warn"}`}>{error ?? notice}</div>
            <button
              onClick={() => (error ? clearError() : useApp.setState({ notice: null }))}
              aria-label="Закрыть"
              className="text-muted hover:text-fg"
            >
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

function FirstRun() {
  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <PageTitle sub="Cyberpunk 2077 · откуда взять Mod Organizer 2">Начало</PageTitle>
      <div className="panel">
        <InstanceSetup />
      </div>
    </div>
  );
}
