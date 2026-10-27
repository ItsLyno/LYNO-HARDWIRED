import { ChevronDown, FolderSearch, TriangleAlert, type LucideIcon } from "lucide-react";
import { useState, type ReactNode } from "react";
import type { GameMismatch } from "../api";
import { useApp } from "../store";
import { Button } from "./Button";
import { UpdateProgress } from "./UpdateProgress";

/** What stands between the player and a working game, on the page itself: a healthy setup shows nothing.
 *  Everything that can wait (updates, the last run's errors) is in the header's `Inbox`. */
export function Notices() {
  const { status, progress, setPage } = useApp();
  return (
    <div className="space-y-3 empty:hidden [&:not(:empty)]:mt-4">
      {progress && <UpdateProgress progress={progress} />}
      {status && !status.gameFound && (
        <Alert tone="bad" icon={FolderSearch} title="Игра не найдена" text="Без папки Cyberpunk 2077 игру не запустить">
          <Button className="h-8 text-[13px]" onClick={() => setPage("settings")}>
            Указать в настройках
          </Button>
        </Alert>
      )}
      {status?.gameMismatch && <GameMismatchAlert m={status.gameMismatch} />}
    </div>
  );
}

/** The game is another patch than the build: frameworks (RED4ext, CET, redscript) are made for one patch, so
 *  after a game update the build often breaks until its own update. Playing stays allowed: the player decides. */
function GameMismatchAlert({ m }: { m: GameMismatch }) {
  const [open, setOpen] = useState(false);
  const yours = m.found ? `патч ${m.found}` : "другая версия";
  const text =
    m.newer === false
      ? `Сборка сделана для патча ${m.expected}, а у вас ${yours}: обновите игру`
      : m.newer
        ? `У вас ${yours}, а сборка сделана для патча ${m.expected}`
        : `Сборка сделана для патча ${m.expected}, а у вас ${yours}`;
  return (
    <Alert
      tone="warn"
      icon={TriangleAlert}
      title={m.newer === false ? "Игра старее сборки" : m.newer ? "Игра обновилась" : "Другая версия игры"}
      text={text}
      below={
        open && (
          <div className="space-y-2 border-t border-line/60 px-5 py-4 text-muted">
            {m.newer === false ? (
              <p>Обновите Cyberpunk 2077 в Steam, GOG или Epic: моды сборки рассчитаны на более новый патч.</p>
            ) : (
              <>
                <p>
                  RED4ext, Cyber Engine Tweaks, redscript и моды на них работают с одним патчем игры. После обновления
                  игры они часто не запускаются, пока не выйдут их новые версии и обновление сборки.
                </p>
                <p>
                  Играть можно, но если игра падает или моды не работают — дождитесь обновления сборки. Чтобы игра не
                  обновлялась без спроса: в Steam — свойства игры → Обновления → «Обновлять игру только при её
                  запуске», в GOG Galaxy и Epic — отключите автообновление игры.
                </p>
              </>
            )}
          </div>
        )
      }
    >
      <Button variant="ghost" className="h-8 text-[13px]" onClick={() => setOpen(!open)} aria-expanded={open}>
        Подробнее
        <ChevronDown size={14} className={`transition-transform ${open ? "rotate-180" : ""}`} />
      </Button>
    </Alert>
  );
}

const tones = {
  bad: { icon: "text-bad", border: "border-bad/30" },
  warn: { icon: "text-warn", border: "border-warn/30" },
};

function Alert(props: { tone: keyof typeof tones; icon: LucideIcon; title: string; text: string; below?: ReactNode; children?: ReactNode }) {
  const t = tones[props.tone];
  return (
    <div role="alert" className={`panel border text-[13px] ${t.border}`}>
      <div className="flex items-center gap-3 px-5 py-3">
        <props.icon size={16} className={`shrink-0 ${t.icon}`} />
        <div className="min-w-0 flex-1">
          <div className="font-medium text-fg">{props.title}</div>
          <div className="text-muted">{props.text}</div>
        </div>
        {props.children}
      </div>
      {props.below}
    </div>
  );
}
