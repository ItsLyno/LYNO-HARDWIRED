import { Download, FolderInput, Layers } from "lucide-react";
import { useState, type ReactNode } from "react";
import { api } from "../api";
import { useApp } from "../store";
import { Button } from "./Button";

/** Where the game's MO2 comes from: the official one, the player's own, or the build's (it brings its own). */
export function InstanceSetup({ build = true }: { build?: boolean }) {
  const { switchInstance, startMo2Setup, progress } = useApp();
  const [dir, setDir] = useState("");
  const busy = !!progress;

  return (
    <div className="divide-y divide-line/60">
      <Option
        icon={<Download size={17} />}
        title="Скачать Mod Organizer 2"
        hint="Последняя официальная версия с GitHub, портативная и сразу настроенная на Cyberpunk 2077. Моды ставятся с Nexus или из архивов."
      >
        <Button variant="primary" disabled={busy} onClick={startMo2Setup}>
          Скачать
        </Button>
      </Option>
      <Option
        icon={<FolderInput size={17} />}
        title="Подключить свой Mod Organizer 2"
        hint="Папка портативного MO2 для Cyberpunk 2077, в которой лежит ModOrganizer.exe. Лаунчер работает с ним как есть."
      >
        <div className="flex gap-2">
          <input
            value={dir}
            spellCheck={false}
            placeholder="D:\Modding\MO2"
            onChange={(e) => setDir(e.target.value)}
            className="h-9 min-w-0 flex-1 rounded-full bg-raised px-4 font-mono text-[13px] outline-none focus:ring-1 focus:ring-neon/50"
          />
          <Button disabled={busy || !dir.trim()} onClick={async () => (await switchInstance(() => api.instanceAdd(dir.trim()))) && setDir("")}>
            Подключить
          </Button>
        </div>
      </Option>
      {build && (
        <Option
          icon={<Layers size={17} />}
          title="Сборка LYNO//HARDWIRED"
          hint="Готовая сборка модов со своим MO2 в отдельной папке. Ваш Mod Organizer 2 она не трогает."
        >
          <Button disabled={busy} onClick={() => switchInstance(api.instanceAddBuild)}>
            Выбрать сборку
          </Button>
        </Option>
      )}
    </div>
  );
}

function Option(props: { icon: ReactNode; title: string; hint: string; children: ReactNode }) {
  return (
    <div className="flex gap-4 px-5 py-5">
      <div className="mt-0.5 text-muted">{props.icon}</div>
      <div className="min-w-0 flex-1">
        <div className="text-sm font-medium">{props.title}</div>
        <div className="mt-0.5 text-[13px] text-muted">{props.hint}</div>
        <div className="mt-3">{props.children}</div>
      </div>
    </div>
  );
}
