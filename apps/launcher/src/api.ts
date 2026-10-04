import { getVersion } from "@tauri-apps/api/app";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";
import { mock } from "./mock";

export interface Settings {
  instanceDir: string;
  gameDir: string | null;
  manifestUrl: string;
}

export interface Status {
  mo2Installed: boolean;
  gameDir: string | null;
  gameFound: boolean;
  installedVersion: string | null;
  modsTotal: number;
  modsEnabled: number;
  gameRunning: boolean;
  mo2Running: boolean;
  updating: boolean;
}

export interface ChangelogEntry {
  version: string;
  date?: string;
  notes: string[];
}

export type ModRow =
  | { kind: "separator"; title: string }
  | {
      kind: "mod";
      id: string;
      name: string;
      title: string | null;
      version: string | null;
      author: string | null;
      nexusUrl: string | null;
      enabled: boolean;
      size: number;
      outdated: boolean;
      installed: boolean;
    };

export interface BuildInfo {
  name: string;
  latestVersion: string;
  installedVersion: string | null;
  gameVersion: string;
  changelog: ChangelogEntry[];
  mods: ModRow[];
  upToDate: boolean;
  changes: number;
  downloadSize: number;
  online: boolean;
}

export interface GameInstall {
  path: string;
  store: string;
}

export type UpdateEvent =
  | { kind: "step"; index: number; total: number; label: string }
  | { kind: "bytes"; done: number; total: number }
  | { kind: "done" };

export interface UpdateFinished {
  ok: boolean;
  error: string | null;
}

export interface LauncherUpdate {
  version: string;
  currentVersion: string;
  notes: string | null;
}

export const api = isTauri()
  ? {
      getSettings: () => invoke<Settings>("get_settings"),
      saveSettings: (settings: Settings) => invoke<void>("save_settings", { settings }),
      detectGames: () => invoke<GameInstall[]>("detect_games"),
      getStatus: () => invoke<Status>("get_status"),
      fetchBuild: () => invoke<BuildInfo>("fetch_build"),
      startUpdate: () => invoke<void>("start_update"),
      cancelUpdate: () => invoke<void>("cancel_update"),
      launchGame: () => invoke<void>("launch_game"),
      openMo2: () => invoke<void>("open_mo2"),
      /** Path of the zip written to the desktop. */
      exportReport: () => invoke<string>("export_report"),
      launcherVersion: () => getVersion(),
      checkLauncherUpdate: () => invoke<LauncherUpdate | null>("check_launcher_update"),
      installLauncherUpdate: () => invoke<void>("install_launcher_update"),
      openUrl: (url: string) => tauriOpenUrl(url),
      onUpdate: (
        progress: (e: UpdateEvent) => void,
        finished: (f: UpdateFinished) => void,
      ): Promise<UnlistenFn> =>
        Promise.all([
          listen<UpdateEvent>("update-progress", (e) => progress(e.payload)),
          listen<UpdateFinished>("update-finished", (e) => finished(e.payload)),
        ]).then((fns) => () => fns.forEach((f) => f())),
    }
  : mock;

export type Api = typeof api;
