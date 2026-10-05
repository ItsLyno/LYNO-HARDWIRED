import { getVersion } from "@tauri-apps/api/app";
import { invoke, isTauri } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { openUrl as tauriOpenUrl } from "@tauri-apps/plugin-opener";
import { mock } from "./mock";

export interface InstanceEntry {
  name: string;
  dir: string;
  /** Installed and updated from the build's manifest; otherwise the player's own MO2. */
  build: boolean;
}

export interface Settings {
  /** The active instance, one of `instances`; none of them before the first setup. */
  instanceDir: string;
  instances: InstanceEntry[];
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

export type Folder = "instance" | "game" | "saves" | "logs" | "downloads" | "overwrite";

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

export interface NexusUser {
  name: string;
  /** Premium accounts get download links from the API; free ones only via "Mod Manager Download" on the site. */
  premium: boolean;
}

export interface NxmHandler {
  /** Registering is possible here (Windows). */
  supported: boolean;
  ours: boolean;
  /** Program that opens nxm links now, when not the launcher (nxmhandler.exe of MO2, Vortex.exe). */
  other: string | null;
}

export interface NexusStatus {
  keySaved: boolean;
  account: NexusUser | null;
  /** The saved key could not be checked or no longer works. */
  accountError: string | null;
  /** "Log in with Nexus" is available. */
  sso: boolean;
  handler: NxmHandler;
}

export interface NewFile {
  fileId: number;
  name: string;
  fileName: string;
  size: number | null;
}

export type ModStatus =
  | { kind: "unknown" }
  | { kind: "upToDate" }
  /** file: null when Nexus doesn't say which file replaces the installed one. */
  | { kind: "update"; file: NewFile | null; version: string | null }
  | { kind: "unavailable" };

export interface ModUpdateRow {
  folder: string;
  game: string;
  modId: number;
  version: string | null;
  /** Under LYNO USER MODS. */
  personal: boolean;
  /** Installed by the build. */
  managed: boolean;
  /** The player's own mod, or any mod in author mode. */
  canUpdate: boolean;
  status: ModStatus;
  /** Files tab on Nexus, at the new file when known. */
  pageUrl: string;
  /** Requirements on its Nexus page the list doesn't meet; empty for a disabled mod. */
  needs: Need[];
}

/** A requirement on a mod's Nexus page that no enabled mod meets. Authors fill these loosely: advice, not a block. */
export interface Need {
  /** A mod of the same game on Nexus; null: a tool or another site, which the launcher can't check. */
  modId: number | null;
  name: string;
  url: string;
  notes: string;
  /** The required mod is installed, but this folder of it is disabled. */
  disabled: string | null;
}

/** Requests left on the Nexus account: a daily allowance, then an hourly one once it is spent. */
export interface RateLimit {
  daily: number | null;
  hourly: number | null;
  dailyLimit: number | null;
  hourlyLimit: number | null;
}

export interface UpdatesView {
  mods: ModUpdateRow[];
  /** Mods without a Nexus page in meta.ini. */
  untracked: string[];
  /** Unix seconds of the oldest check; null when some mod was never checked. */
  checkedAt: number | null;
  rateLimit: RateLimit | null;
}

export type Mo2Reason = "fomod" | "format" | "layout";

export type Outcome =
  | { kind: "installed"; folder: string }
  /** In MO2's downloads until the player drags it onto the list; `replaces`: the installed mod it is another version of. */
  | { kind: "downloaded"; replaces: string | null }
  /** Not an archive the launcher reads: left in MO2's downloads. */
  | { kind: "mo2"; reason: Mo2Reason }
  /** The rest park the job ("waitingMo2", "choosingRoot", "choosing") and never show as done. */
  | { kind: "deferred" }
  | { kind: "manual" }
  | { kind: "fomod" };

export type JobState =
  | { kind: "queued" }
  | { kind: "downloading"; done: number; total: number }
  | { kind: "retry"; attempt: number; delaySecs: number; error: string }
  | { kind: "waiting" }
  /** MO2 or the game is running: installs by itself once they close. */
  | { kind: "waitingMo2" }
  | { kind: "installing" }
  /** No rule recognizes the archive: the player picks the mod's folder (`installRoots`). */
  | { kind: "choosingRoot" }
  /** A FOMOD installer waits for the player's choice (`nexusFomod`). */
  | { kind: "choosing" }
  | { kind: "done"; outcome: Outcome }
  | { kind: "failed"; error: string }
  | { kind: "cancelled" };

/** What installing an archive would replace, asked before the install. */
export interface ArchiveTarget {
  modName: string;
  version: string | null;
  /** The installed mod folder the archive goes over; `null`: a new mod. */
  replaces: string | null;
  installedVersion: string | null;
}

export interface NexusJob {
  id: number;
  /** "file": an archive from MO2's downloads or dropped from Explorer. */
  source: "nexus" | "file";
  game: string;
  modId: number;
  fileId: number;
  title: string | null;
  fileTitle: string | null;
  version: string | null;
  /** Installed mod folder this file replaces. */
  replaces: string | null;
  /** Once done: requirements on the mod's page the list doesn't meet. */
  needs: Need[];
  state: JobState;
}

/** An archive in MO2's downloads. */
export interface DownloadItem {
  fileName: string;
  size: number;
  /** Unix seconds. */
  modified: number;
  modName: string;
  fileTitle: string | null;
  version: string | null;
  modId: number;
  fileId: number;
  installed: boolean;
}

/** A folder of an archive the player may pick as the mod. */
export interface ArchiveRoot {
  /** "" for the archive itself, else "Folder/Sub/". */
  path: string;
  files: number;
  /** Has a game data folder (archive, bin, r6, red4ext…) or loose .archive files at its top. */
  valid: boolean;
}

/** The player's own section of the list (under LYNO USER MODS), in MO2's order. */
export type UserRow =
  | { kind: "separator"; title: string; color: string | null }
  | { kind: "mod"; name: string; enabled: boolean; version: string | null; nexusUrl: string | null };

/** What the game and its tools wrote through MO2's virtual file system. */
export interface OverwriteInfo {
  files: number;
  size: number;
}

/** An entry of MO2's executables list. */
export interface Executable {
  title: string;
  binary: string;
}

export type GroupKind = "exactlyOne" | "atMostOne" | "atLeastOne" | "all" | "any";
export type PluginKind = "required" | "optional" | "recommended" | "notUsable" | "couldBeUsable";

export interface FomodOutline {
  name: string | null;
  /** Installer path of the main image, a key of `FomodWizard.images`. */
  image: string | null;
  steps: {
    name: string;
    groups: { name: string; kind: GroupKind; plugins: { name: string; description: string; image: string | null }[] }[];
  }[];
}

/** Picked plugins by step, group and plugin index; null for a step not seen yet (the installer's defaults). */
export type FomodSelection = (number[][] | null)[];

/** The wizard at a selection, evaluated by the launcher. */
export interface FomodState {
  /** Per step: shown with the options picked on the steps before it. */
  visible: boolean[];
  /** Per step, group and plugin. */
  kinds: PluginKind[][][];
  /** The selection with the installer's rules applied; empty for hidden steps. */
  selection: number[][][];
  /** Per step: every group has an allowed number of picks. */
  valid: boolean[];
}

export interface FomodWizard {
  outline: FomodOutline;
  /** data: URLs by installer path; images the launcher can't show are missing. */
  images: Record<string, string>;
  /** The choice of the installed version this file replaces. */
  previous: FomodSelection | null;
  state: FomodState;
}

export interface NexusHandlers {
  job: (j: NexusJob) => void;
  check: (p: { done: number; total: number }) => void;
  /** A download finished: installed mods changed. */
  changed: () => void;
  /** An nxm link arrived. */
  link: () => void;
  /** Nexus answered: requests left. */
  limits: (l: RateLimit) => void;
}

export interface FileDropHandlers {
  over: (at: { x: number; y: number }) => void;
  drop: (paths: string[], at: { x: number; y: number }) => void;
  leave: () => void;
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
      /** Null for the player's own MO2. */
      fetchBuild: () => invoke<BuildInfo | null>("fetch_build"),
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
      /** Checks the token (null: the saved one) for push access, then turns author mode on. */
      authorEnable: (token: string | null) => invoke<void>("author_enable", { token }),
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
      launchGame: () => invoke<void>("launch_game", { executable: null }),
      /** Another of MO2's executables, by title. */
      launchExecutable: (executable: string) => invoke<void>("launch_game", { executable }),
      /** MO2's executables besides the game the play button starts. */
      executables: () => invoke<Executable[]>("executables"),
      openMo2: () => invoke<void>("open_mo2"),
      /** Path of the zip written to the desktop. */
      exportReport: () => invoke<string>("export_report"),
      launcherVersion: () => getVersion(),
      checkLauncherUpdate: () => invoke<LauncherUpdate | null>("check_launcher_update"),
      installLauncherUpdate: () => invoke<void>("install_launcher_update"),
      openUrl: (url: string) => tauriOpenUrl(url),
      nexusStatus: () => invoke<NexusStatus>("nexus_status"),
      nexusSetKey: (key: string) => invoke<NexusStatus>("nexus_set_key", { key }),
      nexusLogout: () => invoke<void>("nexus_logout"),
      nexusSsoLogin: () => invoke<NexusStatus>("nexus_sso_login"),
      nexusSsoCancel: () => invoke<void>("nexus_sso_cancel"),
      nxmRegister: () => invoke<NxmHandler>("nxm_register"),
      nxmUnregister: () => invoke<NxmHandler>("nxm_unregister"),
      /** What the last check knew, without asking Nexus. */
      nexusUpdates: () => invoke<UpdatesView>("nexus_updates"),
      /** Null when cancelled. */
      nexusCheck: (force: boolean) => invoke<UpdatesView | null>("nexus_check", { force }),
      nexusCheckCancel: () => invoke<void>("nexus_check_cancel"),
      nexusJobs: () => invoke<NexusJob[]>("nexus_jobs"),
      nexusCancelJob: (id: number) => invoke<void>("nexus_cancel_job", { id }),
      nexusClearJobs: () => invoke<void>("nexus_clear_jobs"),
      /** Premium only: downloads the file without a visit to the site. */
      nexusDownload: (game: string, modId: number, fileId: number) => invoke<number>("nexus_download", { game, modId, fileId }),
      nexusLimits: () => invoke<RateLimit | null>("nexus_limits"),
      downloadsRecent: () => invoke<DownloadItem[]>("downloads_recent"),
      /** A file name in MO2's downloads or an absolute path; `after`: the list entry it was dropped below. */
      installArchive: (file: string, after: string | null) => invoke<number>("install_archive", { file, after }),
      archiveTarget: (file: string) => invoke<ArchiveTarget>("archive_target", { file }),
      installRoots: (id: number) => invoke<ArchiveRoot[]>("install_roots", { id }),
      installSetRoot: (id: number, root: string) => invoke<void>("install_set_root", { id, root }),
      userMods: () => invoke<UserRow[]>("user_mods"),
      setUserModEnabled: (folder: string, enabled: boolean) => invoke<void>("set_user_mod_enabled", { folder, enabled }),
      openUserModFolder: (folder: string) => invoke<void>("open_user_mod_folder", { folder }),
      /** The player's own mod, or any in author mode. */
      deleteMod: (folder: string) => invoke<void>("delete_mod", { folder }),
      /** A separator takes its new title. */
      renameMod: (folder: string, name: string) => invoke<void>("rename_mod", { folder, name }),
      /** Within the player's section; `after`: the entry it was dropped below, null for the end. */
      moveUserMod: (folder: string, after: string | null) => invoke<void>("move_user_mod", { folder, after }),
      addSeparator: (title: string, after: string | null) => invoke<void>("add_separator", { title, after }),
      /** `#rrggbb`; null takes the color away. */
      setSeparatorColor: (folder: string, color: string | null) => invoke<void>("set_separator_color", { folder, color }),
      overwriteInfo: () => invoke<OverwriteInfo>("overwrite_info"),
      overwriteToMod: (name: string) => invoke<void>("overwrite_to_mod", { name }),
      clearOverwrite: () => invoke<void>("clear_overwrite"),
      /** From the archive in MO2's downloads, as a queued job; a player's build mod is repaired instead (`startRepair`). */
      reinstallMod: (folder: string) => invoke<number>("reinstall_mod", { folder }),
      instanceSelect: (dir: string) => invoke<void>("instance_select", { dir }),
      /** The player's portable MO2 for Cyberpunk. */
      instanceAdd: (dir: string) => invoke<void>("instance_add", { dir }),
      /** Switches to the build's instance; installing it is `startUpdate`. */
      instanceAddBuild: () => invoke<void>("instance_add_build"),
      /** Downloads the official MO2 into a new instance; reports like an update. */
      startMo2Setup: () => invoke<void>("start_mo2_setup"),
      /** Archives dropped on the window from Explorer, with the drop point in CSS pixels. */
      onFileDrop: (handlers: FileDropHandlers): Promise<UnlistenFn> =>
        getCurrentWebview().onDragDropEvent((e) => {
          const p = e.payload;
          const at = (pos: { x: number; y: number }) => ({ x: pos.x / window.devicePixelRatio, y: pos.y / window.devicePixelRatio });
          if (p.type === "over") handlers.over(at(p.position));
          else if (p.type === "drop") handlers.drop(p.paths, at(p.position));
          else if (p.type === "leave") handlers.leave();
        }),
      nexusFomod: (id: number) => invoke<FomodWizard>("nexus_fomod", { id }),
      nexusFomodEval: (id: number, selection: FomodSelection) => invoke<FomodState>("nexus_fomod_eval", { id, selection }),
      /** Queues the install of the choice; the job reports the outcome. */
      nexusFomodInstall: (id: number, selection: FomodSelection) => invoke<void>("nexus_fomod_install", { id, selection }),
      onNexus: (handlers: NexusHandlers): Promise<UnlistenFn> =>
        Promise.all([
          listen<NexusJob>("nexus-job", (e) => handlers.job(e.payload)),
          listen<{ done: number; total: number }>("nexus-check", (e) => handlers.check(e.payload)),
          listen("nexus-changed", () => handlers.changed()),
          listen("nexus-link", () => handlers.link()),
          listen<RateLimit>("nexus-limits", (e) => handlers.limits(e.payload)),
        ]).then((fns) => () => fns.forEach((f) => f())),
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
