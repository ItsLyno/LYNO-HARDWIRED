import { RotateCw } from "lucide-react";
import { Button } from "../components/Button";
import { StatusDot, type Tone } from "../components/StatusDot";
import { useApp } from "../store";

interface Check {
  title: string;
  detail: string;
  tone: Tone;
  result: string;
}

export function Diagnostics() {
  const { status, refresh } = useApp();

  const checks: Check[] = [
    {
      title: "Mod Organizer 2",
      detail: "ModOrganizer.exe в папке инстанса",
      tone: !status ? "idle" : status.mo2Installed ? "ok" : "bad",
      result: !status ? "—" : status.mo2Installed ? "Найден" : "Не найден",
    },
    {
      title: "Портативный режим",
      detail: "portable.txt рядом с ModOrganizer.exe",
      tone: !status ? "idle" : status.portable ? "ok" : "warn",
      result: !status ? "—" : status.portable ? "Включён" : "Выключен",
    },
    {
      title: "Профиль сборки",
      detail: "profiles/<профиль>/modlist.txt",
      tone: !status ? "idle" : status.profileExists ? "ok" : "warn",
      result: !status ? "—" : status.profileExists ? "Найден" : "Не найден",
    },
    ...[
      ["Версия игры", "Cyberpunk2077.exe совпадает с версией сборки"],
      ["Целостность модов", "Хэши файлов совпадают с манифестом"],
      ["Системные компоненты", "VC++ Redistributable, свободное место"],
      ["Логи после вылета", "RED4ext, Cyber Engine Tweaks, redscript"],
    ].map(([title, detail]) => ({ title, detail, tone: "idle" as Tone, result: "Скоро" })),
  ];

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <div className="flex items-center justify-between">
        <h1 className="text-2xl font-semibold tracking-tight">Диагностика</h1>
        <Button onClick={refresh}>
          <RotateCw size={15} />
          Проверить снова
        </Button>
      </div>
      <ul className="divide-y divide-line rounded-lg border border-line bg-surface">
        {checks.map((c) => (
          <li key={c.title} className="flex items-center gap-4 px-5 py-3.5">
            <StatusDot tone={c.tone} />
            <div className="flex-1">
              <div>{c.title}</div>
              <div className="text-[13px] text-muted">{c.detail}</div>
            </div>
            <div className={`text-[13px] ${c.tone === "idle" ? "text-faint" : ""}`}>{c.result}</div>
          </li>
        ))}
      </ul>
    </div>
  );
}
