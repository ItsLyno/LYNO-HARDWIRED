import { create } from "zustand";
import { api, type BuildInfo, type LauncherUpdate, type Settings, type Status, type UpdateEvent, type VerifyReport } from "./api";

export type Page = "home" | "mods" | "updates" | "release" | "settings";

export interface Progress {
  step: { index: number; total: number; label: string } | null;
  bytes: { done: number; total: number } | null;
  /** Connection lost, reconnecting (attempt number). */
  retry: { attempt: number } | null;
}

const noProgress: Progress = { step: null, bytes: null, retry: null };

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
  clearError: () => set({ error: null }),
}));

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
