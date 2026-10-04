import { create } from "zustand";
import { api, type BuildInfo, type LauncherUpdate, type Settings, type Status, type UpdateEvent } from "./api";

export type Page = "home" | "mods" | "updates" | "settings";

export interface Progress {
  step: { index: number; total: number; label: string } | null;
  bytes: { done: number; total: number } | null;
}

interface AppStore {
  page: Page;
  settings: Settings | null;
  status: Status | null;
  build: BuildInfo | null;
  buildError: string | null;
  progress: Progress | null;
  error: string | null;
  launcherUpdate: LauncherUpdate | null;
  setPage: (page: Page) => void;
  refreshStatus: () => Promise<void>;
  refreshBuild: () => Promise<void>;
  saveSettings: (s: Settings) => Promise<boolean>;
  startUpdate: () => Promise<void>;
  onUpdateEvent: (e: UpdateEvent) => void;
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
    set({ progress: { step: null, bytes: null } });
    try {
      await api.startUpdate();
      await get().refreshStatus();
    } catch (e) {
      set({ progress: null, error: String(e) });
    }
  },
  onUpdateEvent: (e) => {
    const p = get().progress ?? { step: null, bytes: null };
    if (e.kind === "step") set({ progress: { ...p, step: e } });
    if (e.kind === "bytes") set({ progress: { ...p, bytes: e } });
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

/** What the main button should do right now. */
export type PrimaryAction = "loading" | "game" | "install" | "update" | "updating" | "play" | "running";

export function primaryAction(status: Status | null, build: BuildInfo | null, updating: boolean): PrimaryAction {
  if (!status) return "loading";
  if (updating || status.updating) return "updating";
  if (status.gameRunning) return "running";
  if (!status.installedVersion || !status.mo2Installed) return build ? "install" : "loading";
  if (build && !build.upToDate && build.online) return "update";
  if (!status.gameFound) return "game";
  return "play";
}
