import { Gamepad2 } from "lucide-react";
import { api } from "../api";
import { useApp } from "../store";
import { Button } from "./Button";

/** Over the pages while the game runs: MO2's own "locked" window is hidden, this is what stands in for it. */
export function GameRunning() {
  const { status, run } = useApp();
  if (!status?.gameRunning) return null;
  return (
    <div className="fixed inset-x-0 top-14 bottom-12 z-50 flex items-center justify-center bg-bg/80 backdrop-blur-sm">
      <div className="panel flex w-96 flex-col items-center gap-4 p-8 text-center">
        <Gamepad2 size={40} className="text-accent" />
        <div>
          <div className="text-lg font-semibold text-fg">Игра запущена</div>
          <div className="mt-1 text-[13px] text-muted">Моды менять нельзя, пока игра открыта. Лаунчер вернётся, когда вы выйдете.</div>
        </div>
        <Button variant="ghost" onClick={() => run(api.stopGame)}>
          Завершить игру
        </Button>
      </div>
    </div>
  );
}
