import { Gamepad2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api";
import { useApp } from "../store";
import { Button } from "./Button";

function elapsed(startedSecs: number, now: number): string {
  const s = Math.max(0, Math.floor(now / 1000) - startedSecs);
  const h = Math.floor(s / 3600);
  const m = String(Math.floor((s % 3600) / 60)).padStart(2, "0");
  return h > 0 ? `${h}:${m}:${String(s % 60).padStart(2, "0")}` : `${m}:${String(s % 60).padStart(2, "0")}`;
}

/** Over the pages while the game runs: MO2's own "locked" window is hidden, this is what stands in for it.
 *  It can be put aside: the pages that change mods lock themselves, the rest stays usable. */
export function GameRunning() {
  const { status, run } = useApp();
  const [aside, setAside] = useState(false);
  const [now, setNow] = useState(Date.now());
  const running = !!status?.gameRunning;

  useEffect(() => {
    if (!running) {
      setAside(false);
      return;
    }
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [running]);

  if (!status || !running || aside) return null;
  const rows: [string, string][] = [
    ["В игре", status.gameStarted ? elapsed(status.gameStarted, now) : "—"],
    ["Патч игры", status.gamePatch ?? "—"],
    ["Сборка", status.installedVersion ?? "—"],
    ["Модов включено", `${status.modsEnabled} из ${status.modsTotal}`],
  ];
  return (
    <div className="fixed inset-x-0 top-14 bottom-12 z-50 flex items-center justify-center bg-bg/80 backdrop-blur-sm">
      <div className="panel flex w-96 flex-col items-center gap-4 p-8 text-center">
        <Gamepad2 size={40} className="text-accent" />
        <div>
          <div className="text-lg font-semibold text-fg">Игра запущена</div>
          <div className="mt-1 text-[13px] text-muted">
            Моды менять нельзя, пока игра открыта. Можно свернуть это окно и посмотреть лаунчер; моды останутся заблокированными.
          </div>
        </div>
        <dl className="grid w-full grid-cols-[1fr_auto] gap-x-4 gap-y-1 text-left text-[13px]">
          {rows.map(([k, v]) => (
            <div key={k} className="contents">
              <dt className="text-muted">{k}</dt>
              <dd className="font-mono text-fg">{v}</dd>
            </div>
          ))}
        </dl>
        <div className="flex gap-2">
          <Button variant="ghost" onClick={() => setAside(true)}>
            Свернуть
          </Button>
          <Button variant="ghost" onClick={() => run(api.stopGame)}>
            Завершить игру
          </Button>
        </div>
      </div>
    </div>
  );
}
