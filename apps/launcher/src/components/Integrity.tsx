import { Check, Loader2, ShieldCheck, Wrench } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type Damaged } from "../api";
import { plural } from "../format";
import { useApp } from "../store";
import { Button } from "./Button";
import { StatusDot } from "./StatusDot";
import { UpdateProgress } from "./UpdateProgress";

/** Integrity check and repair: the result lists damaged mods, the player picks what to download again. */
export function Integrity() {
  const { build, status, progress, verifyProgress, verifyReport, verify, startRepair } = useApp();
  const [selected, setSelected] = useState<Set<string>>(new Set());

  // Everything is selected by default: a damaged mod is the common case, a changed config the exception.
  useEffect(() => {
    setSelected(new Set(verifyReport?.damaged.map(key) ?? []));
  }, [verifyReport]);

  const busy = !!progress || !!status?.updating;
  const installed = !!status?.installedVersion;
  const titleOf = (d: Damaged) => {
    if (d.id === null) return "Mod Organizer 2";
    const row = build?.mods.find((m) => m.kind === "mod" && m.id === d.id);
    return row?.kind === "mod" ? (row.title ?? row.name) : d.folder;
  };
  const toggle = (k: string) => {
    const next = new Set(selected);
    if (!next.delete(k)) next.add(k);
    setSelected(next);
  };
  const repair = () => {
    const ids = [...selected].filter((k) => k !== BASE);
    void startRepair(ids, selected.has(BASE));
  };

  if (verifyProgress) {
    return <UpdateProgress progress={verifyProgress} onCancel={api.cancelVerify} />;
  }

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
        <div className="rounded-md border border-line bg-bg">
          <div className="border-b border-line px-4 py-2.5 text-[13px]">
            Повреждено: {verifyReport.damaged.length} из {verifyReport.checked}{" "}
            {plural(verifyReport.checked, "мода", "модов", "модов")}
          </div>
          <ul className="divide-y divide-line">
            {verifyReport.damaged.map((d) => (
              <li key={key(d)}>
                <label className="flex cursor-pointer items-start gap-3 px-4 py-2.5 hover:bg-raised/50">
                  <input
                    type="checkbox"
                    checked={selected.has(key(d))}
                    onChange={() => toggle(key(d))}
                    className="mt-0.5 accent-[var(--color-accent)]"
                  />
                  <StatusDot tone="warn" />
                  <span className="min-w-0 flex-1">
                    <span className="block truncate text-sm">{titleOf(d)}</span>
                    <span className="block text-[13px] text-muted">{describe(d)}</span>
                  </span>
                </label>
              </li>
            ))}
          </ul>
          <div className="flex items-center gap-3 border-t border-line px-4 py-3">
            <Button variant="primary" onClick={repair} disabled={busy || selected.size === 0 || !build?.online}>
              {busy ? <Loader2 size={15} className="animate-spin" /> : <Wrench size={15} />}
              Починить выбранные
            </Button>
            {!build?.online && <span className="text-[13px] text-muted">Нет связи с GitHub</span>}
          </div>
        </div>
      )}
    </div>
  );
}

const BASE = "\u0000base";

function key(d: Damaged): string {
  return d.id ?? BASE;
}

function describe(d: Damaged): string {
  switch (d.problem.kind) {
    case "missingFolder":
      return "Папка мода удалена";
    case "changed":
      return "Файлы изменены, удалены или добавлены";
    case "missingFiles": {
      const { count, files } = d.problem;
      const more = count > files.length ? ` и ещё ${count - files.length}` : "";
      return `Нет ${count} ${plural(count, "файла", "файлов", "файлов")}: ${files.join(", ")}${more}`;
    }
  }
}
