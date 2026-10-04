import { invoke, isTauri } from "@tauri-apps/api/core";

export interface Settings {
  instanceDir: string;
  profile: string;
  manifestUrl: string;
}

export interface Status {
  mo2Installed: boolean;
  portable: boolean;
  profileExists: boolean;
  modsTotal: number;
  modsEnabled: number;
  gameRunning: boolean;
  mo2Running: boolean;
}

export interface ChangelogEntry {
  version: string;
  date?: string;
  notes: string[];
}

export interface BuildInfo {
  name: string;
  installedVersion: string | null;
  latestVersion: string;
  gameVersion: string;
  modCount: number;
  changelog: ChangelogEntry[];
}

// In a plain browser (`pnpm dev` without Tauri) the UI runs against mocks
// so screens can be designed and screenshotted without Windows/MO2.
const mock = {
  settings: {
    instanceDir: "C:\\Users\\V\\AppData\\Local\\dev.lyno.hardwired\\instance",
    profile: "LYNO",
    manifestUrl: "https://github.com/ItsLyno/LYNO-HARDWIRED/releases/latest/download/manifest.json",
  } satisfies Settings,
  status: {
    mo2Installed: true,
    portable: true,
    profileExists: true,
    modsTotal: 184,
    modsEnabled: 179,
    gameRunning: false,
    mo2Running: false,
  } satisfies Status,
  build: {
    name: "LYNO//HARDWIRED",
    installedVersion: "1.3.2",
    latestVersion: "1.4.0",
    gameVersion: "2.31",
    modCount: 186,
    changelog: [
      {
        version: "1.4.0",
        date: "2026-10-02",
        notes: [
          "Обновлены Cyber Engine Tweaks и RED4ext под патч 2.31",
          "Добавлен Better Vehicle Handling",
          "Удалён Immersive Traffic — конфликтовал с новым трафиком",
        ],
      },
      {
        version: "1.3.2",
        date: "2026-09-18",
        notes: ["Исправлен порядок загрузки архивов освещения", "Обновлён Nova LUT"],
      },
      {
        version: "1.3.0",
        date: "2026-09-01",
        notes: ["Новый пресет ReShade", "Добавлены 6 модов на одежду", "Обновлён ArchiveXL"],
      },
    ],
  } satisfies BuildInfo,
};

async function call<T>(cmd: string, args?: Record<string, unknown>, fallback?: () => T): Promise<T> {
  if (isTauri()) return invoke<T>(cmd, args);
  await new Promise((r) => setTimeout(r, 150));
  return fallback ? fallback() : (undefined as T);
}

export const api = {
  getSettings: () => call<Settings>("get_settings", undefined, () => mock.settings),
  saveSettings: (settings: Settings) =>
    call<void>("save_settings", { settings }, () => {
      mock.settings = settings;
    }),
  getStatus: () => call<Status>("get_status", undefined, () => mock.status),
  // Manifest fetching lands with the updater; until then the real app has no build info.
  getBuildInfo: async (): Promise<BuildInfo | null> => (isTauri() ? null : mock.build),
  launchGame: () => call<void>("launch_game"),
  openMo2: () => call<void>("open_mo2"),
};
