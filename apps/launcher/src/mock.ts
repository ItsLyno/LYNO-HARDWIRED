// Browser-only stand-in for the Tauri backend (`pnpm dev` without Tauri),
// so screens can be built and screenshotted without Windows or MO2.
// The data is illustrative, not the real build.
import type { BuildInfo, Folder, GameInstall, LauncherUpdate, ModRow, Settings, Status, UpdateEvent, UpdateFinished, VerifyReport } from "./api";

const GB = 1024 ** 3;
const MB = 1024 ** 2;

function mod(name: string, author: string, version: string, size: number, nexusId: number, extra: Partial<ModRow> = {}): ModRow {
  return {
    kind: "mod",
    id: name.toLowerCase().replace(/[^a-z0-9]+/g, "-"),
    name,
    title: null,
    version,
    author,
    nexusUrl: `https://www.nexusmods.com/cyberpunk2077/mods/${nexusId}`,
    enabled: true,
    optional: false,
    size,
    outdated: false,
    installed: true,
    recent: null,
    damaged: false,
    ...extra,
  } as ModRow;
}

const mods: ModRow[] = [
  { kind: "separator", title: "Фреймворки" },
  mod("Cyber Engine Tweaks", "yamashi", "1.35.0", 24 * MB, 107, { outdated: true }),
  mod("RED4ext", "WopsS", "1.27.0", 3 * MB, 2380, { outdated: true }),
  mod("redscript", "jekky", "0.5.27", 6 * MB, 1511),
  mod("ArchiveXL", "psiberx", "1.21.1", 2 * MB, 4198),
  mod("TweakXL", "psiberx", "1.10.6", 2 * MB, 4197),
  mod("Codeware", "psiberx", "1.15.0", 3 * MB, 7780),
  mod("Mod Settings", "jackhumbert", "0.2.11", 1 * MB, 4885),
  { kind: "separator", title: "Графика" },
  mod("Nova LUT", "Kvan7", "2.3", 180 * MB, 2075, { optional: true, recent: "updated" }),
  mod("Ultra Plus", "sammilucia", "5.1.2", 12 * MB, 10490),
  mod("HD Reworked Project", "HalkHogan", "1.0", 9.4 * GB, 7652, { title: "HD Reworked Project — Ultra Quality", optional: true, enabled: false }),
  { kind: "separator", title: "Геймплей" },
  mod("Better Vehicle Handling", "Erok", "2.4", 1 * MB, 3401, { installed: false }),
  mod("Immersive Rippers", "xBaebsae", "1.2", 4 * MB, 9330, { recent: "added" }),
  mod("Never Lose Your Car", "Jelle Bakker", "1.0.3", 1 * MB, 6012, { enabled: false }),
  mod("Appearance Menu Mod", "MxOrcinus", "2.6.3", 420 * MB, 790),
];

let settings: Settings = {
  instanceDir: "C:\\Users\\V\\AppData\\Local\\dev.lyno.hardwired\\instance",
  gameDir: "D:\\SteamLibrary\\steamapps\\common\\Cyberpunk 2077",
  manifestUrl: "https://raw.githubusercontent.com/ItsLyno/LYNO-HARDWIRED/main/build/manifest.json",
};

let status: Status = {
  mo2Installed: true,
  gameDir: settings.gameDir,
  gameFound: true,
  installedVersion: "1.3.2",
  modsTotal: 14,
  modsEnabled: 13,
  gameRunning: false,
  mo2Running: false,
  updating: false,
};

let build: BuildInfo = {
  name: "LYNO//HARDWIRED",
  latestVersion: "1.4.0",
  installedVersion: "1.3.2",
  gameVersion: "2.31",
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
    { version: "1.3.2", date: "2026-09-18", notes: ["Исправлен порядок загрузки архивов освещения", "Обновлён Nova LUT"] },
    { version: "1.3.0", date: "2026-09-01", notes: ["Новый пресет ReShade", "Добавлены 6 модов на одежду", "Обновлён ArchiveXL"] },
  ],
  mods,
  upToDate: false,
  changes: 4,
  repairs: 0,
  downloadSize: 28 * MB,
  online: true,
  lastUpdate: { from: "1.3.0", to: "1.3.2", removed: ["Immersive Traffic"] },
};

const delay = (ms: number) => new Promise((r) => setTimeout(r, ms));
let progress: ((e: UpdateEvent) => void) | null = null;
let finished: ((f: UpdateFinished) => void) | null = null;
let cancelled = false;
let verifyProgress: ((e: UpdateEvent) => void) | null = null;
let verifyCancelled = false;

async function simulateVerify(): Promise<VerifyReport | null> {
  verifyCancelled = false;
  const installed = build.mods.filter((m) => m.kind === "mod" && m.installed);
  const total = installed.reduce((n, m) => n + (m.kind === "mod" ? m.size : 0), 0);
  let done = 0;
  for (const [i, m] of installed.entries()) {
    if (m.kind !== "mod") continue;
    verifyProgress?.({ kind: "step", index: i + 1, total: installed.length, label: m.name });
    for (let t = 0; t < 4; t++) {
      if (verifyCancelled) return null;
      await delay(80);
      done += m.size / 4;
      verifyProgress?.({ kind: "bytes", done, total });
    }
  }
  return {
    checked: installed.length,
    damaged: [
      { id: null, folder: "", problem: { kind: "missingFiles", count: 2, files: ["ModOrganizer.exe", "dlls/QtCore.dll"] } },
      { id: "cyber-engine-tweaks", folder: "Cyber Engine Tweaks", problem: { kind: "changed" } },
      { id: "nova-lut", folder: "Nova LUT", problem: { kind: "missingFolder" } },
    ],
  };
}

async function simulateUpdate() {
  const steps = ["Обновление: Cyber Engine Tweaks", "Обновление: RED4ext", "Установка: Better Vehicle Handling", "Удаление: Immersive Traffic", "Порядок загрузки"];
  const total = build.downloadSize;
  let done = 0;
  for (const [i, label] of steps.entries()) {
    progress?.({ kind: "step", index: i + 1, total: steps.length, label });
    for (let t = 0; t < 10 && i < 3; t++) {
      if (cancelled) return finished?.({ ok: false, error: null });
      await delay(120);
      done = Math.min(total, done + total / 30);
      progress?.({ kind: "bytes", done, total });
    }
  }
  progress?.({ kind: "done" });
  status = { ...status, updating: false, installedVersion: build.latestVersion };
  build = {
    ...build,
    installedVersion: build.latestVersion,
    upToDate: true,
    changes: 0,
    downloadSize: 0,
    mods: build.mods.map((m) => (m.kind === "mod" ? { ...m, outdated: false, installed: true } : m)),
  };
  finished?.({ ok: true, error: null });
}

export const mock = {
  getSettings: async () => settings,
  saveSettings: async (s: Settings) => {
    settings = s;
    status = { ...status, gameDir: s.gameDir, gameFound: !!s.gameDir };
  },
  detectGames: async (): Promise<GameInstall[]> => [{ path: "D:\\SteamLibrary\\steamapps\\common\\Cyberpunk 2077", store: "Steam" }],
  getStatus: async () => status,
  fetchBuild: async () => {
    await delay(200);
    return build;
  },
  startUpdate: async () => {
    cancelled = false;
    status = { ...status, updating: true };
    void simulateUpdate();
  },
  cancelUpdate: async () => {
    cancelled = true;
    status = { ...status, updating: false };
  },
  verifyBuild: () => simulateVerify(),
  cancelVerify: async () => {
    verifyCancelled = true;
  },
  startRepair: async (_ids: string[], _base: boolean) => {
    cancelled = false;
    status = { ...status, updating: true };
    void simulateUpdate();
  },
  setModEnabled: async (id: string, enabled: boolean) => {
    if (status.gameRunning || status.mo2Running) throw new Error("Закройте игру и Mod Organizer 2, чтобы включать и выключать моды");
    build = { ...build, mods: build.mods.map((m) => (m.kind === "mod" && m.id === id ? { ...m, enabled } : m)) };
  },
  openModFolder: async (_id: string) => {},
  openFolder: async (_folder: Folder) => {},
  launchGame: async () => {},
  openMo2: async () => {},
  exportReport: async () => {
    await delay(600);
    return "C:\\Users\\V\\Desktop\\LYNO-report-2026-10-04_18-20-00.zip";
  },
  launcherVersion: async () => "0.1.0",
  checkLauncherUpdate: async (): Promise<LauncherUpdate | null> => ({
    version: "0.2.0",
    currentVersion: "0.1.0",
    notes: "Отчёт для автора сборки, автообновление лаунчера",
  }),
  installLauncherUpdate: async () => {
    await delay(1500);
  },
  openUrl: async (url: string) => {
    window.open(url, "_blank");
  },
  onVerifyProgress: async (p: (e: UpdateEvent) => void) => {
    verifyProgress = p;
    return () => {
      verifyProgress = null;
    };
  },
  onUpdate: async (p: (e: UpdateEvent) => void, f: (r: UpdateFinished) => void) => {
    progress = p;
    finished = f;
    return () => {
      progress = null;
      finished = null;
    };
  },
};
