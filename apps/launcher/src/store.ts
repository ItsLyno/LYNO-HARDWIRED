import { create } from "zustand";
import { api, type BuildInfo, type Settings, type Status } from "./api";

export type Page = "home" | "build" | "diagnostics" | "settings";

interface AppStore {
  page: Page;
  settings: Settings | null;
  status: Status | null;
  build: BuildInfo | null;
  error: string | null;
  setPage: (page: Page) => void;
  refresh: () => Promise<void>;
  saveSettings: (s: Settings) => Promise<void>;
  run: (action: () => Promise<void>) => Promise<void>;
  clearError: () => void;
}

export const useApp = create<AppStore>((set, get) => ({
  page: "home",
  settings: null,
  status: null,
  build: null,
  error: null,
  setPage: (page) => set({ page }),
  refresh: async () => {
    try {
      const [settings, status, build] = await Promise.all([api.getSettings(), api.getStatus(), api.getBuildInfo()]);
      set({ settings, status, build });
    } catch (e) {
      set({ error: String(e) });
    }
  },
  saveSettings: async (settings) => {
    await get().run(() => api.saveSettings(settings));
    await get().refresh();
  },
  run: async (action) => {
    try {
      await action();
    } catch (e) {
      set({ error: String(e) });
    }
  },
  clearError: () => set({ error: null }),
}));

/** What the main button should do right now. */
export type PrimaryAction = "loading" | "setup" | "install" | "update" | "play" | "running";

export function primaryAction(status: Status | null, build: BuildInfo | null): PrimaryAction {
  if (!status) return "loading";
  if (status.gameRunning) return "running";
  if (!status.mo2Installed) return "setup";
  if (!status.profileExists) return "install";
  if (build && build.installedVersion !== build.latestVersion) return "update";
  return "play";
}
