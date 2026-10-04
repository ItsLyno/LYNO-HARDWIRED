import { Check } from "lucide-react";
import { useEffect, useState } from "react";
import type { Settings as SettingsT } from "../api";
import { Button } from "../components/Button";
import { useApp } from "../store";

const fields: { key: keyof SettingsT; label: string; hint: string }[] = [
  {
    key: "instanceDir",
    label: "Папка Mod Organizer 2",
    hint: "Портативный инстанс MO2, которым управляет лаунчер.",
  },
  { key: "profile", label: "Профиль", hint: "Профиль MO2, в котором установлена сборка." },
  { key: "manifestUrl", label: "Манифест сборки", hint: "Адрес файла с описанием актуальной версии сборки." },
];

export function Settings() {
  const { settings, saveSettings } = useApp();
  const [draft, setDraft] = useState<SettingsT | null>(settings);
  const [saved, setSaved] = useState(false);

  useEffect(() => {
    if (settings && !draft) setDraft(settings);
  }, [settings, draft]);
  if (!draft) return null;

  const dirty = JSON.stringify(draft) !== JSON.stringify(settings);

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <h1 className="text-2xl font-semibold tracking-tight">Настройки</h1>
      <div className="divide-y divide-line rounded-lg border border-line bg-surface">
        {fields.map((f) => (
          <label key={f.key} className="block px-5 py-4">
            <div className="text-sm font-medium">{f.label}</div>
            <div className="mt-0.5 text-[13px] text-muted">{f.hint}</div>
            <input
              value={draft[f.key]}
              spellCheck={false}
              onChange={(e) => {
                setDraft({ ...draft, [f.key]: e.target.value });
                setSaved(false);
              }}
              className="mt-2.5 h-9 w-full rounded-md border border-line bg-bg px-3 font-mono text-[13px] outline-none focus:border-muted"
            />
          </label>
        ))}
      </div>
      <div className="flex items-center gap-3">
        <Button
          variant="primary"
          disabled={!dirty}
          onClick={async () => {
            await saveSettings(draft);
            setSaved(true);
          }}
        >
          Сохранить
        </Button>
        {saved && !dirty && (
          <span className="flex items-center gap-1.5 text-[13px] text-muted">
            <Check size={15} className="text-ok" />
            Сохранено
          </span>
        )}
      </div>
      <p className="pt-4 text-xs text-faint">
        LYNO//HARDWIRED 0.1.0 · Моды принадлежат их авторам и скачиваются с Nexus Mods.
      </p>
    </div>
  );
}
