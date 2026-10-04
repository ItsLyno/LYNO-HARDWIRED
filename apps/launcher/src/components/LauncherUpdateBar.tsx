import { Download, Loader2 } from "lucide-react";
import { useState } from "react";
import { api } from "../api";
import { useApp } from "../store";
import { Button } from "./Button";

// The installer closes the launcher and starts the new version, so there is no "done" state.
export function LauncherUpdateBar() {
  const { launcherUpdate, progress, status, run } = useApp();
  const [installing, setInstalling] = useState(false);
  if (!launcherUpdate) return null;
  const busy = !!progress || !!status?.updating;

  const install = () =>
    run(async () => {
      setInstalling(true);
      try {
        await api.installLauncherUpdate();
      } finally {
        setInstalling(false);
      }
    });

  return (
    <div className="mx-6 mt-1 flex shrink-0 items-center gap-4 rounded-full bg-neon/[0.07] py-1.5 pr-1.5 pl-5 text-[13px]">
      <span className="flex-1 truncate">
        Доступна новая версия лаунчера <span className="font-mono font-semibold tabular-nums">{launcherUpdate.version}</span>
        <span className="text-muted"> · сейчас {launcherUpdate.currentVersion}</span>
        {launcherUpdate.notes && <span className="text-muted"> · {launcherUpdate.notes}</span>}
      </span>
      <Button
        className="h-8"
        onClick={install}
        disabled={installing || busy}
        title={busy ? "Дождитесь окончания обновления сборки" : undefined}
      >
        {installing ? <Loader2 size={15} className="animate-spin" /> : <Download size={15} />}
        {installing ? "Загрузка…" : "Обновить лаунчер"}
      </Button>
    </div>
  );
}
