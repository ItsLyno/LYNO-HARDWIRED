import { Check, FolderTree, Loader2, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type ArchiveRoot } from "../api";
import { useApp } from "../store";
import { Button } from "./Button";

/**
 * MO2's manual installer: the archive's layout matched no rule, so the player
 * picks the folder that is the mod, the one holding archive/, bin/, r6/…
 */
export function RootPicker({ jobId, onClose }: { jobId: number; onClose: () => void }) {
  const job = useApp((s) => s.nexusJobs.find((j) => j.id === jobId));
  const { run } = useApp();
  const [roots, setRoots] = useState<ArchiveRoot[] | null>(null);
  const [picked, setPicked] = useState<string | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  useEffect(() => {
    api
      .installRoots(jobId)
      .then((r) => {
        setRoots(r);
        // The shallowest folder MO2's checker would accept, like MO2 suggests.
        setPicked(r.find((x) => x.valid)?.path ?? null);
      })
      .catch((e) => setLoadError(String(e)));
  }, [jobId]);

  useEffect(() => {
    if (job && job.state.kind !== "choosingRoot") onClose();
  }, [job?.state.kind]);

  const install = () =>
    run(async () => {
      if (picked === null) return;
      await api.installSetRoot(jobId, picked);
      onClose();
    });

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-8" role="dialog" aria-modal="true">
      <div className="panel flex max-h-[600px] w-full max-w-2xl flex-col overflow-hidden bg-surface shadow-2xl">
        <div className="flex h-14 shrink-0 items-center gap-3 border-b border-line px-5">
          <FolderTree size={16} className="text-muted" />
          <div className="min-w-0 flex-1">
            <div className="truncate text-sm font-semibold">{job?.title ?? "Архив мода"}</div>
            <div className="text-xs text-muted">Выберите папку, которая и есть мод</div>
          </div>
          <button onClick={onClose} title="Закрыть: выбрать можно позже из загрузок" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <X size={16} />
          </button>
        </div>
        <p className="px-5 pt-4 text-[13px] text-muted">
          Лаунчер не узнал, как устроен архив. Укажите папку, внутри которой лежат папки игры (<span className="font-mono text-xs">archive</span>,{" "}
          <span className="font-mono text-xs">bin</span>, <span className="font-mono text-xs">r6</span>, <span className="font-mono text-xs">red4ext</span>…)
          или файлы <span className="font-mono text-xs">.archive</span>. Подходящие отмечены.
        </p>
        {loadError && <p className="px-5 py-4 text-[13px] text-bad">{loadError}</p>}
        {!roots && !loadError && (
          <div className="flex items-center gap-2 px-5 py-6 text-[13px] text-muted">
            <Loader2 size={15} className="animate-spin" />
            Чтение архива…
          </div>
        )}
        {roots && (
          <ul className="mx-5 my-4 min-h-0 overflow-y-auto rounded-xl bg-bg/40 py-1" role="radiogroup">
            {roots.map((r) => {
              const depth = r.path ? r.path.split("/").length - 1 : 0;
              const name = r.path ? r.path.slice(0, -1).split("/").pop() : "Весь архив";
              const on = picked === r.path;
              return (
                <li key={r.path}>
                  <button
                    role="radio"
                    aria-checked={on}
                    onClick={() => setPicked(r.path)}
                    style={{ paddingLeft: 12 + depth * 18 }}
                    className={`flex w-full items-center gap-2 py-1.5 pr-3 text-left text-[13px] ${on ? "bg-raised" : "hover:bg-raised/60"}`}
                  >
                    <span className={`inline-flex size-3.5 shrink-0 items-center justify-center rounded-full border ${on ? "border-neon bg-neon" : "border-line"}`}>
                      {on && <span className="size-1.5 rounded-full bg-bg" />}
                    </span>
                    <span className={`min-w-0 flex-1 truncate ${r.valid ? "text-fg" : "text-muted"}`}>{name}</span>
                    {r.valid && <Check size={13} className="shrink-0 text-ok" />}
                    <span className="shrink-0 font-mono text-[11px] text-faint tabular-nums">{r.files}</span>
                  </button>
                </li>
              );
            })}
          </ul>
        )}
        <div className="flex h-16 shrink-0 items-center justify-end gap-2 border-t border-line px-5">
          {picked !== null && roots?.find((r) => r.path === picked)?.valid === false && (
            <span className="mr-auto text-xs text-warn">В этой папке нет папок игры: мод может не заработать</span>
          )}
          <Button variant="ghost" onClick={onClose}>
            Позже
          </Button>
          <Button variant="primary" disabled={picked === null} onClick={install}>
            Установить
          </Button>
        </div>
      </div>
    </div>
  );
}
