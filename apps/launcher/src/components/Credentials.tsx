import { Check, KeyRound, Loader2 } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type Secret, type SecretsStatus } from "../api";
import { useApp } from "../store";
import { Button } from "./Button";

/** GitHub token and Nexus key for author mode, kept in Windows Credential Manager. */
export function Credentials({ onChange }: { onChange?: (s: SecretsStatus) => void }) {
  const { settings, run } = useApp();
  const [status, setStatus] = useState<SecretsStatus | null>(null);

  const refresh = () =>
    run(async () => {
      const s = await api.authorSecrets();
      setStatus(s);
      onChange?.(s);
    });
  useEffect(() => {
    void refresh();
  }, []);
  if (!status) return null;

  return (
    <div className="divide-y divide-line/60">
      <SecretRow
        secret="githubToken"
        saved={status.github}
        label="Токен GitHub"
        hint={`Нужен для публикации. Fine-grained token на github.com → Settings → Developer settings, доступ только к ${settings?.authorRepo ?? "репозиторию сборки"}, права Contents: Read and write. Лаунчер проверит его перед сохранением.`}
        onSaved={refresh}
      />
      <SecretRow
        secret="nexusKey"
        saved={status.nexus}
        label="Ключ Nexus API"
        hint="Необязательно. С ним у новых модов в лаунчере появятся авторы и названия с Nexus; без него у старых модов они сохраняются из прошлой версии. Ключ — на nexusmods.com в настройках профиля, API Keys."
        onSaved={refresh}
      />
      <p className="px-5 py-3 text-[13px] text-faint">Хранятся в диспетчере учётных данных Windows, не в файле настроек.</p>
    </div>
  );
}

function SecretRow(props: { secret: Secret; saved: boolean; label: string; hint: string; onSaved: () => Promise<void> }) {
  const { run } = useApp();
  const [value, setValue] = useState("");
  const [busy, setBusy] = useState(false);

  const save = (v: string | null) =>
    run(async () => {
      setBusy(true);
      try {
        await api.authorSetSecret(props.secret, v);
        setValue("");
        await props.onSaved();
      } finally {
        setBusy(false);
      }
    });

  return (
    <div className="px-5 py-4">
      <div className="flex items-center gap-2 text-sm font-medium">
        <KeyRound size={14} className="text-muted" />
        {props.label}
        {props.saved && (
          <span className="flex items-center gap-1 text-[13px] font-normal text-ok">
            <Check size={14} />
            сохранён
          </span>
        )}
      </div>
      <div className="mt-0.5 text-[13px] text-muted">{props.hint}</div>
      <div className="mt-3 flex gap-2">
        <input
          type="password"
          value={value}
          spellCheck={false}
          autoComplete="off"
          placeholder={props.saved ? "Новое значение заменит сохранённое" : ""}
          onChange={(e) => setValue(e.target.value)}
          className="h-9 min-w-0 flex-1 rounded-full bg-raised px-4 font-mono text-[13px] outline-none focus:ring-1 focus:ring-neon/50"
        />
        <Button onClick={() => save(value)} disabled={busy || !value.trim()}>
          {busy && <Loader2 size={15} className="animate-spin" />}
          Сохранить
        </Button>
        {props.saved && (
          <Button variant="ghost" onClick={() => save(null)} disabled={busy}>
            Удалить
          </Button>
        )}
      </div>
    </div>
  );
}
