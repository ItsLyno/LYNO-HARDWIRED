import { Check, Search } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type Settings as SettingsT } from "../api";
import { Button } from "../components/Button";
import { useApp } from "../store";

export function Settings() {
  const { settings, saveSettings, run } = useApp();
  const [draft, setDraft] = useState<SettingsT | null>(settings);
  const [saved, setSaved] = useState(false);
  const [detectNote, setDetectNote] = useState<string | null>(null);

  useEffect(() => {
    if (settings && !draft) setDraft(settings);
  }, [settings, draft]);
  if (!draft) return null;

  const dirty = JSON.stringify(draft) !== JSON.stringify(settings);
  const update = (patch: Partial<SettingsT>) => {
    setDraft({ ...draft, ...patch });
    setSaved(false);
  };

  const detect = () =>
    run(async () => {
      const found = await api.detectGames();
      if (found.length > 0) {
        update({ gameDir: found[0].path });
        setDetectNote(`Найдено: ${found.map((g) => g.store).join(", ")}`);
      } else {
        setDetectNote("Игра не найдена в Steam, GOG и Epic. Укажите папку вручную.");
      }
    });

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <h1 className="text-2xl font-semibold tracking-tight">Настройки</h1>
      <div className="divide-y divide-line rounded-lg border border-line bg-surface">
        <Field label="Папка Cyberpunk 2077" hint="Папка, в которой лежит bin\x64\Cyberpunk2077.exe.">
          <div className="flex gap-2">
            <Input value={draft.gameDir ?? ""} onChange={(v) => update({ gameDir: v || null })} />
            <Button onClick={detect}>
              <Search size={15} />
              Найти
            </Button>
          </div>
          {detectNote && <div className="mt-2 text-[13px] text-muted">{detectNote}</div>}
        </Field>
        <Field label="Папка сборки" hint="Сюда устанавливаются Mod Organizer 2 и моды. Нужно около 1,2× размера сборки свободного места.">
          <Input value={draft.instanceDir} onChange={(v) => update({ instanceDir: v })} />
        </Field>
        <Field label="Манифест сборки" hint="Адрес файла с описанием актуальной версии сборки на GitHub.">
          <Input value={draft.manifestUrl} onChange={(v) => update({ manifestUrl: v })} />
        </Field>
      </div>
      <div className="flex items-center gap-3">
        <Button
          variant="primary"
          disabled={!dirty}
          onClick={async () => {
            if (await saveSettings(draft)) setSaved(true);
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
      <p className="pt-4 text-xs text-faint">LYNO//HARDWIRED 0.1.0 · Моды принадлежат их авторам.</p>
    </div>
  );
}

function Field(props: { label: string; hint: string; children: React.ReactNode }) {
  return (
    <div className="px-5 py-4">
      <div className="text-sm font-medium">{props.label}</div>
      <div className="mt-0.5 text-[13px] text-muted">{props.hint}</div>
      <div className="mt-2.5">{props.children}</div>
    </div>
  );
}

function Input({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <input
      value={value}
      spellCheck={false}
      onChange={(e) => onChange(e.target.value)}
      className="h-9 w-full rounded-md border border-line bg-bg px-3 font-mono text-[13px] outline-none focus:border-muted"
    />
  );
}
