import { create } from "zustand";
import {
  api,
  type AuthorEvent,
  type AuthorJob,
  type BuildInfo,
  type BuiltRelease,
  type DownloadItem,
  type LauncherUpdate,
  type NexusJob,
  type NexusStatus,
  type RateLimit,
  type Settings,
  type Status,
  type UpdateEvent,
  type UpdatesView,
  type UserRow,
  type VerifyReport,
} from "./api";

export type Page = "home" | "mods" | "updates" | "nexus" | "release" | "settings";

export interface Progress {
  step: { index: number; total: number; label: string } | null;
  bytes: { done: number; total: number } | null;
  /** Connection lost, reconnecting (attempt number). */
  retry: { attempt: number } | null;
}

const noProgress: Progress = { step: null, bytes: null, retry: null };

/** A running author build or publish; kept here so it survives switching pages. */
export interface AuthorRun {
  job: AuthorJob;
  progress: Progress;
  /** Latest log lines, oldest first. */
  log: string[];
}

const AUTHOR_LOG_LINES = 6;

interface AppStore {
  page: Page;
  settings: Settings | null;
  status: Status | null;
  build: BuildInfo | null;
  buildError: string | null;
  progress: Progress | null;
  /** Integrity check in progress; kept here so it survives switching pages. */
  verifyProgress: Progress | null;
  verifyReport: VerifyReport | null;
  error: string | null;
  launcherUpdate: LauncherUpdate | null;
  authorRun: AuthorRun | null;
  built: BuiltRelease | null;
  /** Version published in this session, for the success note. */
  published: string | null;
  nexus: NexusStatus | null;
  /** Version tracking of mods from Nexus; null until loaded or without an installed build. */
  nexusUpdates: UpdatesView | null;
  /** Update check in progress: pages asked of all to ask. */
  nexusChecking: { done: number; total: number } | null;
  /** Downloads from nxm links and Premium updates, oldest first. */
  nexusJobs: NexusJob[];
  /** Job whose FOMOD wizard is open. */
  fomodJob: number | null;
  /** Job whose "pick the mod's folder" dialog is open. */
  rootJob: number | null;
  /** Requests left on the Nexus account, from its last answer. */
  nexusLimits: RateLimit | null;
  /** Newest archives of MO2's downloads; null until loaded. */
  downloads: DownloadItem[] | null;
  /** The player's own section of the mod list; null until loaded. */
  userMods: UserRow[] | null;
  /** An archive being dragged from the downloads list; `over`: the list entry it would land below. */
  drag: { file: string; label: string; x: number; y: number; over: DropSpot | null } | null;
  /** Files from Explorer are over the window; `outside`: not over the mod list. */
  fileOver: (DropSpot & { outside?: boolean }) | null;
  setPage: (page: Page) => void;
  refreshStatus: () => Promise<void>;
  refreshBuild: () => Promise<void>;
  saveSettings: (s: Settings) => Promise<boolean>;
  startUpdate: () => Promise<void>;
  setModEnabled: (id: string, enabled: boolean) => Promise<void>;
  verify: () => Promise<void>;
  startRepair: (ids: string[], base: boolean, resetSettings: boolean) => Promise<void>;
  onUpdateEvent: (e: UpdateEvent) => void;
  onVerifyEvent: (e: UpdateEvent) => void;
  onUpdateFinished: (ok: boolean, error: string | null) => void;
  run: (action: () => Promise<void>) => Promise<void>;
  checkLauncherUpdate: () => Promise<void>;
  refreshBuilt: () => Promise<void>;
  startAuthorJob: (job: AuthorJob, start: () => Promise<void>) => Promise<void>;
  onAuthorEvent: (e: AuthorEvent) => void;
  refreshNexus: () => Promise<void>;
  checkNexus: (force: boolean) => Promise<void>;
  onNexusJob: (job: NexusJob) => void;
  setFomodJob: (id: number | null) => void;
  setRootJob: (id: number | null) => void;
  refreshDownloads: () => Promise<void>;
  refreshUserMods: () => Promise<void>;
  /** A file name in MO2's downloads or a path from Explorer. */
  installArchive: (file: string, after: string | null) => Promise<void>;
  clearError: () => void;
}

export const useApp = create<AppStore>((set, get) => ({
  page: "home",
  settings: null,
  status: null,
  build: null,
  buildError: null,
  progress: null,
  verifyProgress: null,
  verifyReport: null,
  error: null,
  launcherUpdate: null,
  authorRun: null,
  built: null,
  published: null,
  nexus: null,
  nexusUpdates: null,
  nexusChecking: null,
  nexusJobs: [],
  fomodJob: null,
  rootJob: null,
  nexusLimits: null,
  downloads: null,
  userMods: null,
  drag: null,
  fileOver: null,
  setPage: (page) => set({ page }),
  refreshStatus: async () => {
    try {
      const [settings, status] = await Promise.all([api.getSettings(), api.getStatus()]);
      set({ settings, status });
    } catch (e) {
      set({ error: String(e) });
    }
  },
  refreshBuild: async () => {
    try {
      set({ build: await api.fetchBuild(), buildError: null });
    } catch (e) {
      set({ buildError: String(e) });
    }
  },
  saveSettings: async (settings) => {
    try {
      await api.saveSettings(settings);
      await get().refreshStatus();
      return true;
    } catch (e) {
      set({ error: String(e) });
      return false;
    }
  },
  startUpdate: async () => {
    set({ progress: noProgress });
    try {
      await api.startUpdate();
      await get().refreshStatus();
    } catch (e) {
      set({ progress: null, error: String(e) });
    }
  },
  // Patches the row in place: refetching the manifest for one switch is a round trip to GitHub.
  setModEnabled: async (id, enabled) => {
    try {
      await api.setModEnabled(id, enabled);
      const build = get().build;
      if (build) {
        set({ build: { ...build, mods: build.mods.map((m) => (m.kind === "mod" && m.id === id ? { ...m, enabled } : m)) } });
      }
      await get().refreshStatus();
    } catch (e) {
      set({ error: String(e) });
    }
  },
  verify: async () => {
    set({ verifyProgress: noProgress, verifyReport: null });
    try {
      const report = await api.verifyBuild();
      set({ verifyProgress: null, verifyReport: report });
    } catch (e) {
      set({ verifyProgress: null, error: String(e) });
    }
  },
  // Repair is an update of the marked mods; its progress shows where update progress does.
  startRepair: async (ids, base, resetSettings) => {
    set({ progress: noProgress });
    try {
      await api.startRepair(ids, base, resetSettings);
      set({ verifyReport: null, page: "updates" });
      await get().refreshStatus();
    } catch (e) {
      set({ progress: null, error: String(e) });
    }
  },
  onUpdateEvent: (e) => {
    const p = get().progress ?? noProgress;
    if (e.kind === "step") set({ progress: { ...p, step: e } });
    // Another worker's resume reports unchanged bytes; only real progress means the connection is back.
    if (e.kind === "bytes") set({ progress: { ...p, bytes: e, retry: e.done !== p.bytes?.done ? null : p.retry } });
    if (e.kind === "retry") set({ progress: { ...p, retry: { attempt: e.attempt } } });
  },
  onVerifyEvent: (e) => {
    const p = get().verifyProgress;
    if (!p) return;
    if (e.kind === "step") set({ verifyProgress: { ...p, step: e } });
    if (e.kind === "bytes") set({ verifyProgress: { ...p, bytes: e } });
  },
  onUpdateFinished: (ok, error) => {
    set({ progress: null, error: error });
    if (ok || error) void get().refreshBuild();
    void get().refreshStatus();
  },
  run: async (action) => {
    try {
      await action();
    } catch (e) {
      set({ error: String(e) });
    }
  },
  // Silent: an unreachable update server must not greet the player with an error.
  checkLauncherUpdate: async () => {
    try {
      set({ launcherUpdate: await api.checkLauncherUpdate() });
    } catch {
      set({ launcherUpdate: null });
    }
  },
  refreshBuilt: async () => {
    try {
      set({ built: await api.authorBuilt() });
    } catch (e) {
      set({ error: String(e) });
    }
  },
  startAuthorJob: async (job, start) => {
    set({ authorRun: { job, progress: noProgress, log: [] }, published: null });
    try {
      await start();
    } catch (e) {
      set({ authorRun: null, error: String(e) });
    }
  },
  onAuthorEvent: (e) => {
    const run = get().authorRun;
    if (e.kind === "finished") {
      set({ authorRun: null, error: e.error });
      if (e.job === "build") void get().refreshBuilt();
      if (e.job === "publish" && e.ok) {
        set({ built: null, published: get().built?.version ?? "", verifyReport: null });
        void get().refreshBuild();
        void get().refreshStatus();
      }
      return;
    }
    if (!run) return;
    if (e.kind === "step") set({ authorRun: { ...run, progress: { ...run.progress, step: e, bytes: e.label === run.progress.step?.label ? run.progress.bytes : null } } });
    if (e.kind === "bytes") set({ authorRun: { ...run, progress: { ...run.progress, bytes: e } } });
    if (e.kind === "log") set({ authorRun: { ...run, log: [...run.log, e.line].slice(-AUTHOR_LOG_LINES) } });
  },
  // Local only: the account check and what the last update check knew. Asking Nexus is `checkNexus`.
  refreshNexus: async () => {
    try {
      const [nexus, jobs] = await Promise.all([api.nexusStatus(), api.nexusJobs()]);
      set({ nexus, nexusJobs: jobs });
    } catch (e) {
      set({ error: String(e) });
    }
    try {
      set({ nexusUpdates: await api.nexusUpdates() });
    } catch {
      // No installed build yet: nothing to track.
      set({ nexusUpdates: null });
    }
  },
  checkNexus: async (force) => {
    set({ nexusChecking: { done: 0, total: 0 } });
    try {
      const view = await api.nexusCheck(force);
      if (view) set({ nexusUpdates: view });
    } catch (e) {
      set({ error: String(e) });
    } finally {
      set({ nexusChecking: null });
    }
  },
  onNexusJob: (job) => {
    const jobs = get().nexusJobs;
    const was = jobs.find((j) => j.id === job.id)?.state.kind;
    set({ nexusJobs: jobs.some((j) => j.id === job.id) ? jobs.map((j) => (j.id === job.id ? job : j)) : [...jobs, job] });
    // The player just clicked "Mod Manager Download": the installer's questions come up by themselves.
    const open = get().fomodJob === null && get().rootJob === null;
    if (job.state.kind === "choosing" && was !== "choosing" && open) set({ fomodJob: job.id });
    if (job.state.kind === "choosingRoot" && was !== "choosingRoot" && open) set({ rootJob: job.id });
    if (job.state.kind === "done" && was !== "done") {
      void get().refreshDownloads();
      void get().refreshUserMods();
    }
  },
  setFomodJob: (id) => set({ fomodJob: id }),
  setRootJob: (id) => set({ rootJob: id }),
  // Quiet: without an installed build there is no downloads folder or list to show.
  refreshDownloads: async () => {
    try {
      set({ downloads: await api.downloadsRecent() });
    } catch {
      set({ downloads: [] });
    }
  },
  refreshUserMods: async () => {
    try {
      set({ userMods: await api.userMods() });
    } catch {
      set({ userMods: [] });
    }
  },
  installArchive: async (file, after) => {
    try {
      await api.installArchive(file, after);
    } catch (e) {
      set({ error: String(e) });
    }
  },
  clearError: () => set({ error: null }),
}));

/** Where a dropped archive lands: below this entry of the list (`after`), `null` for the end of the player's section. */
export interface DropSpot {
  after: string | null;
}

/** The drop spot under a point of the window: rows of the mod list carry `data-drop-after`. */
export function dropSpotAt(x: number, y: number): DropSpot | null {
  const el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop-after], [data-drop-zone]");
  if (!el) return null;
  return { after: el.dataset.dropAfter ?? null };
}

export function isJobActive(job: NexusJob): boolean {
  return !["done", "failed", "cancelled"].includes(job.state.kind);
}

/** The pending update only downloads damaged files again, the version stays the same. */
export function isRepairOnly(build: BuildInfo | null): boolean {
  return !!build && build.repairs > 0 && build.installedVersion === build.latestVersion;
}

/** What the main button should do right now. */
export type PrimaryAction = "loading" | "game" | "install" | "update" | "updating" | "play" | "running";

/** In author mode the instance is ahead of the published build on purpose: no update is offered. */
export function primaryAction(status: Status | null, build: BuildInfo | null, updating: boolean, authorMode = false): PrimaryAction {
  if (!status) return "loading";
  if (updating || status.updating) return "updating";
  if (status.gameRunning) return "running";
  if (!status.installedVersion || !status.mo2Installed) return build ? "install" : "loading";
  if (build && !build.upToDate && build.online && !authorMode) return "update";
  if (!status.gameFound) return "game";
  return "play";
}
