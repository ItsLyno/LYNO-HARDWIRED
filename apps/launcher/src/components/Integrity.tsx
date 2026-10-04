import { Check, Loader2, ShieldCheck, SlidersHorizontal, Wrench } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type Damaged, type Files } from "../api";
import { plural } from "../format";
import { useApp } from "../store";
import { Button } from "./Button";
import { StatusDot } from "./StatusDot";
import { UpdateProgress } from "./UpdateProgress";

/** Integrity check and repair: the result lists damaged mods, the player picks what to download again. */
export function Integrity() {
  const { build, status, progress, verifyProgress, verifyReport, verify, startRepair } = useApp();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [resetSettings, setResetSettings] = useState(false);

  // Everything is selected by default: a repair keeps the player's settings, so it only costs the download.
  useEffect(() => {
    setSelected(new Set(verifyReport?.damaged.map(key) ?? []));
    setResetSettings(false);
  }, [verifyReport]);

  const busy = !!progress || !!status?.updating;
  const installed = !!status?.installedVersion;
  const titleOf = (id: string | null, folder: string) => {
    if (id === null) return "Mod Organizer 2";
    const row = build?.mods.find((m) => m.kind === "mod" && m.id === id);
    return row?.kind === "mod" ? (row.title ?? row.name) : folder;
  };
  const settingsOf = (id: string | null) => verifyReport?.customized.find((c) => c.id === id)?.files ?? null;
  const toggle = (k: string) => {
    const next = new Set(selected);
    if (!next.delete(k)) next.add(k);
    setSelected(next);
  };
  const repair = () => {
    const ids = [...selected].filter((k) => k !== BASE);
    void startRepair(ids, selected.has(BASE), resetSettings);
  };

  if (verifyProgress) {
    return <UpdateProgress progress={verifyProgress} onCancel={api.cancelVerify} />;
  }

  const damagedIds = new Set(verifyReport?.damaged.map((d) => d.id) ?? []);
  const tunedOnly = verifyReport?.customized.filter((c) => !damagedIds.has(c.id)) ?? [];

  return (
    <div className="space-y-3">
      <Button onClick={verify} disabled={busy || !installed} title={installed ? undefined : "Сборка не установлена"}>
        <ShieldCheck size={15} />
        Проверить файлы
      </Button>

      {verifyReport && verifyReport.damaged.length === 0 && (
        <div className="flex items-center gap-2 text-[13px] text-muted">
          <Check size={15} className="text-ok" />
          Все файлы на месте: {verifyReport.checked} {plural(verifyReport.checked, "мод", "мода", "модов")} в порядке
        </div>
      )}

      {verifyReport && verifyReport.damaged.length > 0 && (
        <div className="overflow-hidden rounded-xl border border-line bg-bg/60">
          <div className="border-b border-line px-4 py-2.5 text-[13px]">
            Повреждено: {verifyReport.damaged.length} из {verifyReport.checked}{" "}
            {plural(verifyReport.checked, "мода", "модов", "модов")}
          </div>
          <ul className="divide-y divide-line/60">
            {verifyReport.damaged.map((d) => {
              const settings = settingsOf(d.id);
              return (
                <li key={key(d)}>
                  <label className="flex cursor-pointer items-start gap-3 px-4 py-2.5 hover:bg-raised/50">
                    <input
                      type="checkbox"
                      checked={selected.has(key(d))}
                      onChange={() => toggle(key(d))}
                      className="mt-0.5 accent-[var(--color-accent)]"
                    />
                    <span className="mt-1">
                      <StatusDot tone="warn" />
                    </span>
                    <span className="min-w-0 flex-1 space-y-0.5">
                      <span className="block truncate text-sm">{titleOf(d.id, d.folder)}</span>
                      {describe(d).map((line) => (
                        <span key={line} className="block text-[13px] break-words text-muted">
                          {line}
                        </span>
                      ))}
                      {settings && (
                        <span className="block truncate text-[13px] text-faint" title={settings.sample.join("\n")}>
                          {resetSettings ? "Настройки вернутся к настройкам сборки" : "Ваши настройки сохранятся"}:{" "}
                          {list(settings)}
                        </span>
                      )}
                    </span>
                  </label>
                </li>
              );
            })}
          </ul>
          <div className="space-y-3 border-t border-line px-4 py-3">
            <label className="flex cursor-pointer items-start gap-2.5 text-[13px] text-muted">
              <input
                type="checkbox"
                checked={resetSettings}
                onChange={(e) => setResetSettings(e.target.checked)}
                className="mt-0.5 accent-[var(--color-accent)]"
              />
              <span>
                Сбросить и настройки этих модов к настройкам сборки. Отметьте, если мод перестал работать после того, как вы
                его настраивали.
              </span>
            </label>
            <div className="flex items-center gap-3">
              <Button variant="primary" onClick={repair} disabled={busy || selected.size === 0 || !build?.online}>
                {busy ? <Loader2 size={15} className="animate-spin" /> : <Wrench size={15} />}
                Починить выбранные
              </Button>
              {!build?.online && <span className="text-[13px] text-muted">Нет связи с GitHub</span>}
            </div>
          </div>
        </div>
      )}

      {tunedOnly.length > 0 && (
        <div className="overflow-hidden rounded-xl border border-line bg-bg/60">
          <div className="flex items-center gap-2 border-b border-line px-4 py-2.5 text-[13px]">
            <SlidersHorizontal size={14} className="text-muted" />
            Изменены настройки: {tunedOnly.length} {plural(tunedOnly.length, "мод", "мода", "модов")}
            <span className="text-muted">· это не повреждение</span>
          </div>
          <ul className="divide-y divide-line/60">
            {tunedOnly.map((c) => (
              <li key={c.id} className="px-4 py-2" title={c.files.sample.join("\n")}>
                <span className="block truncate text-sm">{titleOf(c.id, c.folder)}</span>
                <span className="block truncate text-[13px] text-muted">{list(c.files)}</span>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}

const BASE = "\u0000base";

function key(d: Damaged): string {
  return d.id ?? BASE;
}

function describe(d: Damaged): string[] {
  switch (d.problem.kind) {
    case "missingFolder":
      return ["Папка мода удалена"];
    case "changed":
      return [
        "Файлы изменены. Мод поставлен старой версией лаунчера, подробностей нет: это могут быть и ваши настройки, они сохранятся.",
      ];
    case "files": {
      const { missing, changed, added } = d.problem;
      const lines: string[] = [];
      if (missing.count > 0) lines.push(`Нет ${missing.count} ${plural(missing.count, "файла", "файлов", "файлов")}: ${list(missing)}`);
      if (changed.count > 0) lines.push(`Изменено ${changed.count} ${plural(changed.count, "файл", "файла", "файлов")}: ${list(changed)}`);
      if (added.count > 0) lines.push(`Лишние файлы (${added.count}): ${list(added)}`);
      return lines;
    }
  }
}

/** File names, not full paths: the paths are long and the name is what the player recognizes. */
function list(files: Files): string {
  const names = files.sample.map((p) => p.slice(p.lastIndexOf("/") + 1));
  const more = files.count > names.length ? ` и ещё ${files.count - names.length}` : "";
  return names.join(", ") + more;
}
