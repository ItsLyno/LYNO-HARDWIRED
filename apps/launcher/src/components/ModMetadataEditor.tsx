import { Loader2, X } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { api, type AuthorMetadata, type MetadataRow } from "../api";
import { Button } from "./Button";

export function ModMetadataEditor({ folder, locked, onClose, onSaved }: {
  folder: string;
  locked: boolean;
  onClose: () => void;
  onSaved: () => Promise<void>;
}) {
  const [rows, setRows] = useState<MetadataRow[]>([]);
  const [value, setValue] = useState<AuthorMetadata | null>(null);
  const [query, setQuery] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const closeButton = useRef<HTMLButtonElement>(null);

  useEffect(() => {
    let active = true;
    closeButton.current?.focus();
    void api.authorMetadata().then((data) => {
      if (!active) return;
      setRows(data);
      const row = data.find((r) => r.folder === folder);
      if (row) setValue(row.metadata);
      else setError("Мод больше не входит в сборку. Обновите список.");
    }).catch((e) => active && setError(String(e)));
    return () => { active = false; };
  }, [folder]);

  const save = async () => {
    if (!value || busy || locked) return;
    setBusy(true);
    setError(null);
    try {
      await api.authorSaveMetadata(folder, value);
      await onSaved();
      onClose();
    } catch (e) {
      setError(String(e));
    } finally {
      setBusy(false);
    }
  };
  const patch = (change: Partial<AuthorMetadata>) => setValue((v) => v && { ...v, ...change });
  const candidates = rows.filter((r) => r.folder !== folder && `${r.folder} ${r.metadata.title} ${r.metadata.id}`.toLowerCase().includes(query.toLowerCase()));
  const missing = value?.requires.filter((id) => !rows.some((r) => r.metadata.id === id)) ?? [];

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/70 p-8" role="dialog" aria-modal="true" aria-labelledby="metadata-title"
      onMouseDown={(e) => e.target === e.currentTarget && !busy && onClose()}
      onKeyDown={(e) => {
        if (e.key === "Escape" && !busy) { e.stopPropagation(); onClose(); }
        if (e.key === "Tab") {
          const buttons = [...e.currentTarget.querySelectorAll<HTMLElement>("button:not(:disabled), input:not(:disabled)")];
          const first = buttons[0]; const last = buttons.at(-1);
          if (e.shiftKey && document.activeElement === first) { e.preventDefault(); last?.focus(); }
          else if (!e.shiftKey && document.activeElement === last) { e.preventDefault(); first?.focus(); }
        }
      }}>
      <div className="panel flex max-h-full w-full max-w-2xl flex-col overflow-hidden bg-surface shadow-2xl">
        <div className="flex shrink-0 items-center gap-3 border-b border-line px-5 py-4">
          <div className="min-w-0 flex-1">
            <h2 id="metadata-title" className="text-sm font-semibold">Метаданные и зависимости</h2>
            <p className="truncate text-xs text-muted">{folder}</p>
          </div>
          <button ref={closeButton} disabled={busy} onClick={onClose} aria-label="Закрыть" className="rounded-full p-2 text-muted hover:bg-raised"><X size={16} /></button>
        </div>
        <div className="min-h-0 space-y-4 overflow-y-auto p-5 text-[13px]">
          {error && <p role="alert" className="text-bad">{error}</p>}
          {!value && !error && <p className="text-muted">Загрузка метаданных…</p>}
          {value && <>
            <label className="block space-y-1"><span className="text-muted">Постоянный ID</span>
              <input readOnly value={value.id} className="w-full rounded-xl bg-raised px-3 py-2 font-mono text-xs text-muted" />
              <span className="block text-xs text-faint">Ссылки на мод сохранятся при переименовании папки.</span>
            </label>
            {([['title', 'Название в лаунчере'], ['author', 'Автор мода'], ['description', 'Краткое описание']] as const).map(([key, label]) => (
              <label key={key} className="block space-y-1"><span>{label}</span>
                <input value={value[key]} disabled={busy || locked} onChange={(e) => patch({ [key]: e.target.value })}
                  className="w-full rounded-xl bg-raised px-3 py-2 outline-none focus:ring-1 focus:ring-neon/50" />
              </label>
            ))}
            <label className="flex items-start gap-2.5">
              <input type="checkbox" checked={value.core} disabled={busy || locked} onChange={(e) => patch({ core: e.target.checked })} className="mt-0.5 accent-[var(--color-accent)]" />
              <span>Обязательная основа сборки<span className="mt-1 block text-xs text-muted">Игрок не сможет выключить или удалить этот мод. Без отметки мод нужен только пока включены моды, которые от него зависят. Эта настройка заменяет правило разделителя для данного мода.</span></span>
            </label>
            <fieldset disabled={busy || locked} className="space-y-2">
              <legend className="mb-1 font-medium">Для работы этого мода обязательны</legend>
              <p className="text-xs text-muted">Отметьте только необходимые моды. Варианты, рекомендации и требования со страницы Nexus не становятся обязательными автоматически.</p>
              <input aria-label="Поиск зависимости" placeholder="Найти мод сборки…" value={query} onChange={(e) => setQuery(e.target.value)} className="w-full rounded-xl bg-raised px-3 py-2 outline-none focus:ring-1 focus:ring-neon/50" />
              <div className="max-h-52 overflow-y-auto rounded-xl border border-line p-2">
                {candidates.length === 0 && <p className="p-2 text-muted">Модов не найдено</p>}
                {candidates.map((r) => <label key={r.metadata.id} className="flex items-center gap-2.5 rounded-lg p-2 hover:bg-raised">
                  <input type="checkbox" checked={value.requires.includes(r.metadata.id)} onChange={(e) => patch({ requires: e.target.checked ? [...value.requires, r.metadata.id] : value.requires.filter((id) => id !== r.metadata.id) })} className="accent-[var(--color-accent)]" />
                  <span className="min-w-0"><span className="block truncate">{r.metadata.title || r.folder}</span><span className="block truncate font-mono text-[11px] text-faint">{r.metadata.id}</span></span>
                </label>)}
              </div>
              {missing.map((id) => <label key={id} className="flex items-center gap-2 text-bad"><input type="checkbox" checked onChange={() => patch({ requires: value.requires.filter((r) => r !== id) })} />Отсутствует в сборке: {id} — снимите отметку, чтобы удалить связь</label>)}
            </fieldset>
          </>}
        </div>
        <div className="flex shrink-0 items-center justify-end gap-2 border-t border-line px-5 py-4">
          <Button variant="ghost" onClick={onClose} disabled={busy}>Отмена</Button>
          <Button variant="primary" onClick={save} disabled={!value || busy || locked}>{busy && <Loader2 size={14} className="animate-spin" />}Сохранить</Button>
        </div>
      </div>
    </div>
  );
}
