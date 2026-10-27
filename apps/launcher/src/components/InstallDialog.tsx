import { PackagePlus, X } from "lucide-react";
import { useEffect, useState } from "react";
import type { ArchiveTarget } from "../api";
import { useApp } from "../store";
import { Button } from "./Button";

/**
 * Like MO2, every archive install asks for the mod's name first. The name of
 * the installed mod the archive is another version of goes over that mod; any
 * other installs next to it: a variant of the same Nexus page (a different
 * body, an option) looks like a new version to the launcher.
 */
export function InstallDialog() {
  const ask = useApp((s) => s.installAsks[0]);
  // A fresh dialog per archive: the name starts as that archive's.
  return ask ? <Ask key={`${ask.file}\n${ask.after}`} target={ask.target} /> : null;
}

function Ask({ target }: { target: ArchiveTarget }) {
  const answerInstall = useApp((s) => s.answerInstall);
  const build = useApp((s) => s.build);
  const userMods = useApp((s) => s.userMods);
  const [name, setName] = useState(target.replaces ?? target.folder);

  useEffect(() => {
    const esc = (e: KeyboardEvent) => e.key === "Escape" && answerInstall(null);
    window.addEventListener("keydown", esc);
    return () => window.removeEventListener("keydown", esc);
  }, []);

  const trimmed = name.trim();
  const replacing = target.replaces !== null && trimmed === target.replaces;
  const same = target.installedVersion !== null && target.installedVersion === target.version;
  // Another mod's folder isn't replaced: the backend numbers the new one.
  const taken =
    !replacing &&
    [...(build?.mods ?? []), ...(userMods ?? [])].some((m) => m.kind === "mod" && m.name.toLowerCase() === trimmed.toLowerCase());
  const submit = () => trimmed && answerInstall(trimmed);
  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-8" role="dialog" aria-modal="true">
      <div className="panel flex w-full max-w-md flex-col overflow-hidden bg-surface shadow-2xl">
        <div className="flex h-14 shrink-0 items-center gap-3 border-b border-line px-5">
          <PackagePlus size={16} className="text-muted" />
          <div className="min-w-0 flex-1">
            <div className="text-sm font-semibold">Установка мода</div>
            <div className="truncate text-xs text-muted">
              {target.modName}
              {target.version && <span className="ml-1.5 font-mono">{target.version}</span>}
            </div>
          </div>
          <button onClick={() => answerInstall(null)} aria-label="Отмена" className="inline-flex size-8 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg">
            <X size={16} />
          </button>
        </div>
        <div className="space-y-2 px-5 py-4 text-[13px] text-muted">
          <label className="block space-y-1.5">
            <span>Название</span>
            <input
              autoFocus
              value={name}
              onChange={(e) => setName(e.target.value)}
              onFocus={(e) => e.target.select()}
              onKeyDown={(e) => e.key === "Enter" && submit()}
              spellCheck={false}
              placeholder="Название мода"
              className="h-9 w-full rounded-lg bg-raised px-3 text-sm text-fg outline-none placeholder:text-faint focus:ring-1 focus:ring-neon/50"
            />
          </label>
          {replacing ? (
            <>
              <p>
                {same ? "Эта версия уже установлена: мод переустановится из архива." : "Мод уже установлен и заменится версией из архива:"}
              </p>
              {!same && (
                <p className="font-mono text-xs">
                  <span>{target.installedVersion ?? "неизвестна"}</span>
                  <span className="px-2 text-faint">→</span>
                  <span className="text-accent">{target.version ?? "неизвестна"}</span>
                </p>
              )}
              <p>Файлы мода заменятся целиком, место в списке и включённость сохранятся. Другой вариант мода? Дайте ему своё название.</p>
              {target.build && (
                <p className="text-fg">
                  Это мод сборки. Ваша версия займёт его место, и обновления сборки её больше не тронут. Вернуть версию сборки
                  можно в меню мода.
                </p>
              )}
            </>
          ) : target.replaces ? (
            <p>Установится отдельным модом, «{target.replaces}» останется как есть.</p>
          ) : null}
          {taken && <p className="text-warn">Мод с таким названием уже есть: к новому добавится номер, например «{trimmed} (2)».</p>}
        </div>
        <div className="flex justify-end gap-2 border-t border-line px-5 py-3">
          <Button variant="ghost" onClick={() => answerInstall(null)}>
            Отмена
          </Button>
          <Button variant="primary" disabled={!trimmed} onClick={submit}>
            {replacing ? (same ? "Переустановить" : "Заменить") : "Установить"}
          </Button>
        </div>
      </div>
    </div>
  );
}
