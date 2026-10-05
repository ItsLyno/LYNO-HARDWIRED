import { create } from "zustand";
import {
  api,
  type ArchiveTarget,
  type AuthorEvent,
  type AuthorJob,
  type BuildInfo,
  type BuiltRelease,
  type DownloadItem,
  type InstanceEntry,
  type LauncherUpdate,
  type NexusJob,
  type NexusStatus,
  type OverwriteInfo,
  type RateLimit,
  type Settings,
  type Status,
  type UpdateEvent,
  type UpdatesView,
  type UserRow,
  type VerifyReport,
} from "./api";

export type Page = "mods" | "release" | "settings";

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
  /** A warning that isn't an error, shown like one (a just-installed mod's missing requirements). */
  notice: string | null;
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
  /** An archive being dragged from the downloads list, or with `move` an entry of the player's section
   *  (`file`: its folder; `files`: every selected one, in list order); `over`: the list entry it would land below. */
  drag: { file: string; files?: string[]; label: string; x: number; y: number; over: DropSpot | null; move?: boolean } | null;
  /** MO2's `overwrite/`; null until loaded. */
  overwrite: OverwriteInfo | null;
  /** Files from Explorer are over the window; `outside`: not over the mod list. */
  fileOver: (DropSpot & { outside?: boolean }) | null;
  setPage: (page: Page) => void;
  refreshStatus: () => Promise<void>;
  refreshBuild: () => Promise<void>;
  saveSettings: (s: Settings) => Promise<boolean>;
  startUpdate: () => Promise<void>;
  /** Official MO2 into a new instance; progress shows as an update's. */
  startMo2Setup: () => Promise<void>;
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
  /** New versions of the build and the launcher; Nexus mods are checked from the mod list (`checkNexus`). */
  checkUpdates: () => Promise<void>;
  updatesChecking: boolean;
  /** Last update check of all three, ms since epoch. */
  lastCheck: number | null;
  onNexusJob: (job: NexusJob) => void;
  setFomodJob: (id: number | null) => void;
  setRootJob: (id: number | null) => void;
  refreshDownloads: () => Promise<void>;
  refreshUserMods: () => Promise<void>;
  /** Archives that would go over an installed mod, waiting for the player's yes (first one asked). */
  replaceAsks: ReplaceAsk[];
  /** A file name in MO2's downloads or a path from Explorer. Over an installed mod only after `answerReplace`. */
  installArchive: (file: string, after: string | null) => Promise<void>;
  /** Runs an `instance*` call, then reloads everything that belongs to the instance. */
  switchInstance: (op: () => Promise<void>) => Promise<boolean>;
  answerReplace: (replace: boolean) => void;
  clearError: () => void;
}

export const useApp = create<AppStore>((set, get) => ({
  page: "mods",
  settings: null,
  status: null,
  build: null,
  buildError: null,
  progress: null,
  verifyProgress: null,
  verifyReport: null,
  error: null,
  notice: null,
  launcherUpdate: null,
  authorRun: null,
  built: null,
  published: null,
  nexus: null,
  nexusUpdates: null,
  nexusChecking: null,
  lastCheck: readLastCheck(),
  updatesChecking: false,
  nexusJobs: [],
  fomodJob: null,
  rootJob: null,
  nexusLimits: null,
  downloads: null,
  userMods: null,
  drag: null,
  overwrite: null,
  fileOver: null,
  replaceAsks: [],
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
  startMo2Setup: async () => {
    set({ progress: noProgress });
    try {
      await api.startMo2Setup();
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
      set({ verifyReport: null, page: "mods" });
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
    void get().refreshUserMods();
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
  // Both keep their errors to themselves (`buildError`, no launcher update): a timed check never pops up a toast.
  checkUpdates: async () => {
    if (get().updatesChecking) return;
    set({ updatesChecking: true });
    await Promise.all([activeInstance(get().settings)?.build ? get().refreshBuild() : null, get().checkLauncherUpdate()]);
    const at = Date.now();
    set({ updatesChecking: false, lastCheck: at });
    try {
      localStorage.setItem(LAST_CHECK_KEY, String(at));
    } catch {
      // Without storage the next start just checks again.
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
      const missing = job.needs.filter((n) => n.modId !== null);
      if (missing.length > 0) {
        const names = missing.map((n) => (n.disabled ? `${n.name} (выключен)` : n.name)).join(", ");
        set({ notice: `«${job.title ?? "Мод"}» требует: ${names}. Без них мод может не работать.` });
      }
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
    // overwrite/ is the bottom of the player's list in MO2 too.
    const [userMods, overwrite] = await Promise.all([api.userMods().catch(() => []), api.overwriteInfo().catch(() => null)]);
    set({ userMods, overwrite });
  },
  installArchive: async (file, after) => {
    // Unknown target (unreadable archive, no build): the install itself reports why.
    const target = await api.archiveTarget(file).catch(() => null);
    if (target?.replaces) {
      set({ replaceAsks: [...get().replaceAsks, { file, after, target }] });
      return;
    }
    try {
      await api.installArchive(file, after);
    } catch (e) {
      set({ error: String(e) });
    }
  },
  answerReplace: (replace) => {
    const [ask, ...rest] = get().replaceAsks;
    set({ replaceAsks: rest });
    if (ask && replace) void api.installArchive(ask.file, ask.after).catch((e) => set({ error: String(e) }));
  },
  switchInstance: async (op) => {
    try {
      await op();
    } catch (e) {
      set({ error: String(e) });
      return false;
    }
    set({ build: null, buildError: null, verifyReport: null, nexusUpdates: null });
    await get().refreshStatus();
    void get().refreshBuild();
    void get().refreshNexus();
    void get().refreshDownloads();
    void get().refreshUserMods();
    return true;
  },
  clearError: () => set({ error: null }),
}));

const LAST_CHECK_KEY = "lyno.lastUpdateCheck";

function readLastCheck(): number | null {
  try {
    return Number(localStorage.getItem(LAST_CHECK_KEY)) || null;
  } catch {
    return null;
  }
}

/** A timed update check is due: the interval passed since the last one, also across restarts. */
export function checkDue(hours: number, last: number | null): boolean {
  return hours > 0 && (last === null || Date.now() - last >= hours * 3_600_000);
}

/** The active instance; none before the first setup. */
export function activeInstance(settings: Settings | null): InstanceEntry | null {
  return settings?.instances.find((i) => i.dir === settings.instanceDir) ?? null;
}

export interface ReplaceAsk {
  file: string;
  after: string | null;
  target: ArchiveTarget;
}

/** Where a dropped archive lands: below this entry of the list (`after`), `null` for the end of the player's section.
 *  `build`: a row of the build's section, where a row of the player's can't go. */
export interface DropSpot {
  after: string | null;
  build?: boolean;
}

/** The drop spot under a point of the window: rows of the mod list carry `data-drop-after`. Like in MO2, the gap
 *  nearest to the pointer: over the upper half of a row it is the one above it, so the line doesn't jump a whole
 *  row while the pointer crosses one. */
export function dropSpotAt(x: number, y: number): DropSpot | null {
  let el = document.elementFromPoint(x, y)?.closest<HTMLElement>("[data-drop-after], [data-drop-zone]");
  if (!el) return null;
  const r = el.getBoundingClientRect();
  if (el.dataset.dropAfter !== undefined && y < r.top + r.height / 2) {
    const rows = [...document.querySelectorAll<HTMLElement>("[data-drop-after]")];
    el = rows[rows.indexOf(el) - 1] ?? el;
  }
  return { after: el.dataset.dropAfter ?? null, build: el.dataset.dropBuild !== undefined };
}

export function isJobActive(job: NexusJob): boolean {
  return !["done", "failed", "cancelled"].includes(job.state.kind);
}

/** The pending update only downloads damaged files again, the version stays the same. */
export function isRepairOnly(build: BuildInfo | null): boolean {
  return !!build && build.repairs > 0 && build.installedVersion === build.latestVersion;
}

/** What the main button should do right now; "setup": no MO2 to start yet. */
export type PrimaryAction = "loading" | "setup" | "game" | "install" | "update" | "updating" | "play" | "running";

/** In author mode the instance is ahead of the published build on purpose: no update is offered. */
export function primaryAction(status: Status | null, build: BuildInfo | null, updating: boolean, settings: Settings | null): PrimaryAction {
  if (!status || !settings) return "loading";
  if (updating || status.updating) return "updating";
  if (status.gameRunning) return "running";
  const instance = activeInstance(settings);
  if (!instance || (!instance.build && !status.mo2Installed)) return "setup";
  if (instance.build && (!status.installedVersion || !status.mo2Installed)) return build ? "install" : "loading";
  const authorMode = settings.authorMode;
  if (build && !build.upToDate && build.online && !authorMode) return "update";
  if (!status.gameFound) return "game";
  return "play";
}
