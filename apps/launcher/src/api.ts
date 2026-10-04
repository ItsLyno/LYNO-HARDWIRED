import { getVersion } from "@tauri-apps/api/app";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";
import { mock } from "./mock";

export interface Settings {
  instanceDir: string;
  gameDir: string | null;
  manifestUrl: string;
  /** The author releases from this instance: updates and repairs are off. */
  authorMode: boolean;
  /** GitHub repository the author publishes to, owner/name. */
  authorRepo: string;
  /** Where builds are packed; null: release-out in the launcher's data folder. */
  authorOutDir: string | null;
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
  | { kind: "separator"; title: string; color: string | null }
  | {
      kind: "mod";
      id: string;
      name: string;
      title: string | null;
      version: string | null;
      author: string | null;
      nexusUrl: string | null;
      /** For an installed optional mod, the player's choice. */
      enabled: boolean;
      /** The player may switch it on or off; false for core mods (frameworks, libraries). */
      optional: boolean;
      size: number;
      outdated: boolean;
      installed: boolean;
      /** Changed by the last update (see `BuildInfo.lastUpdate`). */
      recent: "added" | "updated" | null;
      /** Marked for repair; the next update downloads it again. */
      damaged: boolean;
    };

export interface LastUpdate {
  from: string;
  to: string;
  /** Folder names of mods the update removed. */
  removed: string[];
}

export type Folder = "instance" | "game" | "saves" | "logs";

export interface BuildInfo {
  name: string;
  latestVersion: string;
  installedVersion: string | null;
  gameVersion: string;
  changelog: ChangelogEntry[];
  mods: ModRow[];
  upToDate: boolean;
  changes: number;
  /** Of `changes`: damaged mods and MO2 itself to download again. */
  repairs: number;
  downloadSize: number;
  /** Of downloadSize: already downloaded by an interrupted update. */
  downloaded: number;
  online: boolean;
  lastUpdate: LastUpdate | null;
}

export interface GameInstall {
  path: string;
  store: string;
}

export type UpdateEvent =
  | { kind: "step"; index: number; total: number; label: string }
  | { kind: "bytes"; done: number; total: number }
  /** Connection lost; downloads retry on their own. Cleared by the next progress. */
  | { kind: "retry"; attempt: number; delaySecs: number; error: string }
  | { kind: "done" };

export interface UpdateFinished {
  ok: boolean;
  error: string | null;
}

export interface Files {
  count: number;
  /** The first few paths. */
  sample: string[];
}

export type Problem =
  | { kind: "missingFolder" }
  /** Installed by an older launcher, no per-file record: changed settings can't be told from damage. */
  | { kind: "changed" }
  | { kind: "files"; missing: Files; changed: Files; added: Files };

export interface Damaged {
  /** Manifest mod id; null for MO2 and its config (the base package). */
  id: string | null;
  folder: string;
  problem: Problem;
}

export interface Customized {
  id: string;
  folder: string;
  files: Files;
}

export interface VerifyReport {
  checked: number;
  damaged: Damaged[];
  /** Mods with settings files the player changed: not damage, a repair keeps them. */
  customized: Customized[];
}

/** The author's mod list against the installed build. Folder names. */
export interface Pending {
  added: string[];
  removed: string[];
  /** [old, new] folder names of the same mod. */
  renamed: [string, string][];
  /** Switched on or off: the build ships the author's state as the default. */
  toggled: string[];
  reordered: boolean;
  /** Under LYNO USER MODS: not shipped. */
  personal: string[];
}

export type Secret = "githubToken" | "nexusKey";

export interface SecretsStatus {
  github: boolean;
  nexus: boolean;
}

export interface BuiltRelease {
  version: string;
  gameVersion: string;
  notes: string[];
  /** New or changed packages. Empty when `restored`. */
  repacked: { name: string; changed: boolean; size: number }[];
  uploadSize: number;
  mods: number;
  warnings: string[];
  /** Read back after a restart: only the version and notes are known. */
  restored: boolean;
}

export type AuthorJob = "build" | "publish";

export type AuthorEvent =
  | { kind: "step"; index: number; total: number; label: string }
  | { kind: "bytes"; done: number; total: number }
  | { kind: "log"; line: string }
  | { kind: "finished"; job: AuthorJob; ok: boolean; error: string | null };

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
      /** Null when cancelled. */
      verifyBuild: () => invoke<VerifyReport | null>("verify_build"),
      cancelVerify: () => invoke<void>("cancel_verify"),
      startRepair: (ids: string[], base: boolean, resetSettings: boolean) =>
        invoke<void>("start_repair", { ids, base, resetSettings }),
      setModEnabled: (id: string, enabled: boolean) => invoke<void>("set_mod_enabled", { id, enabled }),
      authorChanges: () => invoke<Pending>("author_changes"),
      /** Records the published build as installed: the author has just released it from this instance. */
      authorAdopt: () => invoke<void>("author_adopt"),
      authorSecrets: () => invoke<SecretsStatus>("author_secrets"),
      /** null forgets the secret. A GitHub token is checked before it is saved. */
      authorSetSecret: (secret: Secret, value: string | null) => invoke<void>("author_set_secret", { secret, value }),
      authorBuilt: () => invoke<BuiltRelease | null>("author_built"),
      authorBuild: (version: string, gameVersion: string, notes: string[]) =>
        invoke<void>("author_build", { version, gameVersion, notes }),
      authorPublish: () => invoke<void>("author_publish"),
      authorCancel: () => invoke<void>("author_cancel"),
      onAuthorEvent: (handler: (e: AuthorEvent) => void): Promise<UnlistenFn> =>
        listen<AuthorEvent>("author-event", (e) => handler(e.payload)),
      openModFolder: (id: string) => invoke<void>("open_mod_folder", { id }),
      openFolder: (folder: Folder) => invoke<void>("open_folder", { folder }),
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
      onVerifyProgress: (progress: (e: UpdateEvent) => void): Promise<UnlistenFn> =>
        listen<UpdateEvent>("verify-progress", (e) => progress(e.payload)),
    }
  : mock;

export type Api = typeof api;
