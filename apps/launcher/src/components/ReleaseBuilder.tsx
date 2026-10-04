import { CircleCheck, Hammer, Rocket, TriangleAlert } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api";
import { formatBytes, plural } from "../format";
import { useApp } from "../store";
import { Button } from "./Button";
import { UpdateProgress } from "./UpdateProgress";

/** "1.3.2" → "1.3.3"; empty when the last part isn't a number. */
function nextVersion(v: string | null): string {
  const m = v?.match(/^(.*?)(\d+)$/);
  return m ? `${m[1]}${Number(m[2]) + 1}` : "";
}

/** Build form → packing progress → review → publish progress. */
export function ReleaseBuilder({ canPublish }: { canPublish: boolean }) {
  const { status, build, authorRun, built, published, refreshBuilt, startAuthorJob } = useApp();
  const installed = status?.installedVersion ?? null;
  const [form, setForm] = useState({ version: nextVersion(installed), gameVersion: build?.gameVersion ?? "", notes: "" });
  const [editing, setEditing] = useState(false);

  useEffect(() => {
    void refreshBuilt();
  }, [refreshBuilt]);
  useEffect(() => {
    setForm((f) => ({ ...f, gameVersion: f.gameVersion || build?.gameVersion || "" }));
  }, [build?.gameVersion]);

  if (authorRun) {
    return (
      <div className="space-y-3">
        <div className="text-[13px] text-muted">
          {authorRun.job === "build"
            ? "Упаковка. Неизменённые моды не перечитываются; отмена сработает после текущего мода, а следующая сборка продолжит с того же места."
            : "Публикация. Загруженные части не загружаются повторно: после обрыва просто опубликуйте ещё раз."}
        </div>
        <UpdateProgress progress={authorRun.progress} onCancel={api.authorCancel} />
        {authorRun.log.length > 0 && (
          <ul className="space-y-0.5 font-mono text-xs text-muted">
            {authorRun.log.map((l, i) => (
              <li key={i} className="truncate" title={l}>
                {l}
              </li>
            ))}
          </ul>
        )}
      </div>
    );
  }

  if (built && !editing) {
    return (
      <div className="space-y-4">
        <div>
          <div className="text-sm">Версия {built.version} собрана</div>
          <div className="mt-0.5 font-mono text-xs text-muted tabular-nums">
            {built.restored
              ? `собрана в прошлый раз · ${built.mods} ${plural(built.mods, "мод", "мода", "модов")}`
              : `${built.mods} ${plural(built.mods, "мод", "мода", "модов")} · патч игры ${built.gameVersion} · к загрузке ${formatBytes(built.uploadSize)}`}
          </div>
        </div>

        {built.repacked.length > 0 && (
          <ul className="divide-y divide-line/60 overflow-hidden rounded-xl bg-bg/60">
            {built.repacked.map((r) => (
              <li key={r.name} className="flex items-baseline gap-3 px-4 py-2 text-[13px]">
                <span className="min-w-0 flex-1 truncate">{r.name === "base" ? "Mod Organizer 2 и профиль" : r.name}</span>
                <span className={r.changed ? "text-muted" : "text-accent"}>{r.changed ? "изменён" : "новый"}</span>
                <span className="w-20 text-right font-mono text-xs text-muted tabular-nums">{formatBytes(r.size)}</span>
              </li>
            ))}
          </ul>
        )}
        {!built.restored && built.repacked.length === 0 && (
          <div className="text-[13px] text-muted">Все моды взяты из прошлых версий: изменится только список модов и описание.</div>
        )}
        {built.repacked.length > 0 && (
          <p className="text-[13px] text-faint">
            Если здесь мод, который вы не трогали, значит в нём изменился файл (обычно настройки). Посмотрите «Файлы модов»
            выше, прежде чем публиковать.
          </p>
        )}

        {built.warnings.length > 0 && (
          <ul className="space-y-1">
            {built.warnings.map((w) => (
              <li key={w} className="flex gap-2 text-[13px] break-words text-warn">
                <TriangleAlert size={14} className="mt-0.5 shrink-0" />
                {w}
              </li>
            ))}
          </ul>
        )}

        {built.notes.length > 0 && (
          <ul className="list-inside list-disc text-[13px] text-muted">
            {built.notes.map((n) => (
              <li key={n}>{n}</li>
            ))}
          </ul>
        )}

        <div className="flex flex-wrap items-center gap-3">
          <Button variant="primary" disabled={!canPublish} onClick={() => startAuthorJob("publish", api.authorPublish)}>
            <Rocket size={15} />
            Опубликовать {built.version}
          </Button>
          <Button onClick={() => setEditing(true)}>Собрать заново</Button>
          {!canPublish && <span className="text-[13px] text-muted">Сначала сохраните токен GitHub ниже</span>}
        </div>
        <p className="text-[13px] text-faint">
          Сначала загрузятся новые части и лаунчер проверит, что скачивается каждая часть сборки. Только после этого
          игроки увидят новую версию.
        </p>
      </div>
    );
  }

  const notes = form.notes.split("\n").map((n) => n.trim()).filter(Boolean);
  const submit = () => {
    setEditing(false);
    void startAuthorJob("build", () => api.authorBuild(form.version, form.gameVersion, notes));
  };

  return (
    <div className="space-y-4">
      {published && (
        <div className="flex items-start gap-2 text-[13px]">
          <CircleCheck size={15} className="mt-0.5 shrink-0 text-ok" />
          <span>
            Версия {published} опубликована. Лаунчеры игроков увидят её при следующей проверке; GitHub может несколько минут
            отдавать старый манифест.
          </span>
        </div>
      )}
      <div className="grid grid-cols-2 gap-3">
        <Labeled label="Версия сборки">
          <TextInput value={form.version} onChange={(version) => setForm({ ...form, version })} />
        </Labeled>
        <Labeled label="Патч игры">
          <TextInput value={form.gameVersion} onChange={(gameVersion) => setForm({ ...form, gameVersion })} />
        </Labeled>
      </div>
      <Labeled label="Что изменилось — по строке на пункт, игроки увидят это в лаунчере">
        <textarea
          value={form.notes}
          rows={4}
          onChange={(e) => setForm({ ...form, notes: e.target.value })}
          className="w-full rounded-2xl bg-raised px-4 py-2.5 text-[13px] outline-none focus:ring-1 focus:ring-neon/50"
        />
      </Labeled>
      <div className="flex items-center gap-3">
        <Button variant="primary" onClick={submit} disabled={!form.version.trim() || !form.gameVersion.trim()}>
          <Hammer size={15} />
          Собрать {form.version}
        </Button>
        {editing && (
          <Button variant="ghost" onClick={() => setEditing(false)}>
            Отмена
          </Button>
        )}
      </div>
      <p className="text-[13px] text-faint">
        Закройте игру и MO2. Сборка упаковывает только новые и изменённые моды; моды под LYNO USER MODS в неё не попадают.
      </p>
    </div>
  );
}

function Labeled({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <label className="block">
      <span className="mb-1.5 block text-[13px] text-muted">{label}</span>
      {children}
    </label>
  );
}

function TextInput({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  return (
    <input
      value={value}
      spellCheck={false}
      onChange={(e) => onChange(e.target.value)}
      className="h-9 w-full rounded-full bg-raised px-4 font-mono text-[13px] outline-none focus:ring-1 focus:ring-neon/50"
    />
  );
}
