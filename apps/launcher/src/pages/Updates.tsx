import { Download, RefreshCw, RotateCw, Wrench } from "lucide-react";
import { Button } from "../components/Button";
import { Changelog } from "../components/Changelog";
import { PageTitle } from "../components/PageTitle";
import { Section } from "../components/Section";
import { StatusDot } from "../components/StatusDot";
import { UpdateProgress } from "../components/UpdateProgress";
import { formatBytes, plural } from "../format";
import { isRepairOnly, useApp } from "../store";

export function Updates() {
  const { build, buildError, status, progress, refreshBuild, startUpdate } = useApp();
  const installed = status?.installedVersion ?? null;
  const busy = !!progress || !!status?.updating;
  const repair = isRepairOnly(build);

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <div className="flex items-center justify-between">
        <PageTitle>Обновления</PageTitle>
        <Button onClick={refreshBuild} disabled={busy}>
          <RotateCw size={15} />
          Проверить
        </Button>
      </div>

      <div className="grid grid-cols-3 gap-3">
        <Stat label="Установлена" value={installed ?? "—"} />
        <Stat label="Последняя" value={build?.latestVersion ?? "—"} />
        <Stat label="Патч игры" value={build?.gameVersion ?? "—"} />
      </div>

      {progress ? (
        <UpdateProgress progress={progress} />
      ) : (
        <div className="panel flex items-center gap-4 px-5 py-4">
          <StatusDot tone={!build ? "bad" : build.upToDate ? "ok" : "warn"} />
          <div className="flex-1 text-[13px]">
            {!build ? (
              <span className="text-muted">{buildError ?? "Загрузка…"}</span>
            ) : !build.online ? (
              <span className="text-muted">Нет связи с GitHub. Показана установленная версия.</span>
            ) : build.upToDate ? (
              "Установлена последняя версия"
            ) : (
              <>
                {repair ? "Нужно восстановить повреждённые файлы" : installed ? "Доступно обновление" : "Сборка ещё не установлена"}
                <div className="mt-1 font-mono text-xs text-muted tabular-nums">
                  {build.changes} {plural(build.changes, "изменение", "изменения", "изменений")} ·{" "}
                  {formatBytes(build.downloadSize)} к загрузке
                </div>
              </>
            )}
          </div>
          {build?.online && !build.upToDate && (
            <Button variant="primary" onClick={startUpdate} disabled={busy}>
              {repair ? <Wrench size={15} /> : installed ? <RefreshCw size={15} /> : <Download size={15} />}
              {repair ? "Починить" : installed ? "Обновить" : "Установить"}
            </Button>
          )}
        </div>
      )}

      <p className="text-[13px] leading-relaxed text-muted">
        Сборка скачивается с GitHub. При обновлении загружаются только изменившиеся моды; прерванная загрузка
        продолжится с того же места. Моды, добавленные вами в MO2 вручную, не затрагиваются.
      </p>

      {build && build.changelog.length > 0 && (
        <Section title="История изменений">
          <Changelog entries={build.changelog} installed={installed} />
        </Section>
      )}
    </div>
  );
}

function Stat({ label, value }: { label: string; value: string }) {
  return (
    <div className="panel px-5 py-4">
      <div className="label">{label}</div>
      <div className="mt-2 font-mono text-xl font-semibold tabular-nums">{value}</div>
    </div>
  );
}
