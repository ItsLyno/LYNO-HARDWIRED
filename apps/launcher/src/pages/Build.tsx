import { Button } from "../components/Button";
import { Changelog } from "../components/Changelog";
import { Section } from "../components/Section";
import { useApp } from "../store";

export function Build() {
  const { build } = useApp();
  const updateAvailable = !!build && build.installedVersion !== build.latestVersion;

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <h1 className="text-2xl font-semibold tracking-tight">Сборка</h1>

      <div className="grid grid-cols-3 gap-px overflow-hidden rounded-lg border border-line bg-line">
        <Stat label="Установлена" value={build?.installedVersion ?? "—"} />
        <Stat label="Последняя" value={build?.latestVersion ?? "—"} />
        <Stat label="Модов в сборке" value={build ? String(build.modCount) : "—"} />
      </div>

      <div className="flex items-center gap-4 rounded-lg border border-line bg-surface px-5 py-4">
        <div className="flex-1 text-[13px] text-muted">
          {updateAvailable
            ? "Моды скачиваются с Nexus Mods через ваш аккаунт. С Premium загрузка идёт автоматически, без Premium нужно подтвердить каждый файл на сайте."
            : build
              ? "Установлена последняя версия."
              : "Укажите адрес манифеста сборки в настройках."}
        </div>
        <Button variant="primary" disabled title="Установщик появится на следующем этапе">
          {build?.installedVersion ? "Обновить" : "Установить"}
        </Button>
      </div>

      {build && (
        <Section title="История изменений">
          <Changelog entries={build.changelog} installed={build.installedVersion} />
        </Section>
      )}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="bg-surface px-5 py-4">
      <div className="text-xs text-muted">{label}</div>
      <div className="mt-1 text-xl font-semibold tabular-nums">{value}</div>
    </div>
  );
}
