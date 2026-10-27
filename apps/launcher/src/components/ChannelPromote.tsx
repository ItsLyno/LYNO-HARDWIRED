import { Rocket } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type Channels } from "../api";
import { useApp } from "../store";
import { Button } from "./Button";

/** What beta and release hold, and the button that releases the beta build to everyone. */
export function ChannelPromote({ canPublish }: { canPublish: boolean }) {
  const { authorRun, startAuthorJob } = useApp();
  const [channels, setChannels] = useState<Channels | null>(null);
  const [failed, setFailed] = useState<string | null>(null);

  // Again after every job: a publish moves beta, a promote moves release.
  useEffect(() => {
    if (!canPublish || authorRun) return;
    api.authorChannels().then(
      (c) => {
        setChannels(c);
        setFailed(null);
      },
      (e) => setFailed(String(e)),
    );
  }, [canPublish, authorRun]);

  if (!canPublish) return <p className="px-5 pb-5 text-[13px] text-muted">Нужен токен GitHub: версии каналов читаются через него.</p>;
  if (failed) return <p className="px-5 pb-5 text-[13px] text-warn">{failed}</p>;
  if (!channels) return <p className="px-5 pb-5 text-[13px] text-muted">Загрузка…</p>;

  const waiting = channels.beta !== null && channels.beta !== channels.release;
  return (
    <div className="space-y-3 px-5 pb-5">
      <dl className="grid grid-cols-[auto_1fr] gap-x-6 gap-y-1 text-[13px]">
        <dt className="text-muted">Бета</dt>
        <dd className="font-mono tabular-nums">{channels.beta ?? "—"}</dd>
        <dt className="text-muted">Релиз</dt>
        <dd className="font-mono tabular-nums">{channels.release ?? "—"}</dd>
      </dl>
      {waiting ? (
        <Button variant="primary" disabled={!!authorRun} onClick={() => startAuthorJob("promote", api.authorPromote)}>
          <Rocket size={15} />
          Выпустить {channels.beta} в релиз
        </Button>
      ) : (
        <p className="text-[13px] text-muted">В релизе та же версия, что в бете.</p>
      )}
      <p className="text-[13px] text-faint">
        Копируется только манифест: части сборки уже лежат в GitHub и не загружаются заново. Лаунчер ещё раз проверит, что
        каждая часть скачивается.
      </p>
    </div>
  );
}
