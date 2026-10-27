import { Check, FileSearch, History, ListTree, PackageCheck, RotateCw } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type Damaged, type Pending, type Versions } from "../api";
import { Button } from "../components/Button";
import { ChannelPromote } from "../components/ChannelPromote";
import { Credentials } from "../components/Credentials";
import { list } from "../components/Integrity";
import { PageTitle } from "../components/PageTitle";
import { ReleaseBuilder } from "../components/ReleaseBuilder";
import { Section } from "../components/Section";
import { UpdateProgress } from "../components/UpdateProgress";
import { plural } from "../format";
import { useApp } from "../store";

/**
 * Author mode: what the next release changes against the installed (= last published) build.
 * The mod list is compared right away; edited files need the full integrity check.
 */
export function Release() {
  const { status, build, progress, verifyProgress, verifyReport, verify, run, refreshStatus, refreshBuild, startRevert } = useApp();
  const [pending, setPending] = useState<Pending | null>(null);
  const [canPublish, setCanPublish] = useState(false);
  const [versions, setVersions] = useState<Versions | null>(null);
  const [revertTo, setRevertTo] = useState("");
  const [confirmRevert, setConfirmRevert] = useState(false);
  const installed = status?.installedVersion ?? null;

  const refresh = () => run(async () => setPending(await api.authorChanges()));
  const adopt = () =>
    run(async () => {
      await api.authorAdopt();
      useApp.setState({ verifyReport: null });
      await Promise.all([refreshStatus(), refreshBuild()]);
    });
  const revertedFrom = versions?.revertedFrom ?? null;
  const published = build?.online && build.latestVersion !== installed && !revertedFrom ? build.latestVersion : null;
  useEffect(() => {
    if (installed) void refresh();
  }, [installed]);
  useEffect(() => {
    setConfirmRevert(false);
    if (installed && canPublish) void run(async () => setVersions(await api.authorVersions()));
  }, [installed, canPublish]);
  const edited = verifyReport?.damaged.filter((d) => d.id !== null).map((d) => d.folder) ?? [];
  const revertBuild = versions?.builds.find((b) => b.commit === revertTo) ?? versions?.builds[0] ?? null;

  const titleOf = (id: string | null, folder: string) => {
    if (id === null) return "Mod Organizer 2 и профиль";
    const row = build?.mods.find((m) => m.kind === "mod" && m.id === id);
    return row?.kind === "mod" ? (row.title ?? row.name) : folder;
  };

  if (!installed) {
    return (
      <div className="mx-auto max-w-3xl space-y-6">
        <PageTitle>Выпуск</PageTitle>
        <p className="text-[13px] text-muted">
          Сборка не установлена. Выключите режим автора, установите опубликованную версию и включите его снова.
        </p>
      </div>
    );
  }

  const listChanges = pending ? listLines(pending) : [];

  return (
    <div className="mx-auto max-w-3xl space-y-6">
      <PageTitle sub={`изменения с версии ${installed}`}>Выпуск</PageTitle>

      {published && (
        <div className="panel flex items-center gap-4 px-5 py-4">
          <div className="flex-1 text-[13px]">
            Опубликована версия {published}, а установленной считается {installed}.
            <div className="mt-1 text-muted">
              Если вы выпустили её из этой папки, примите её: лаунчер ничего не скачает, а изменения будут считаться от
              новой версии. Если её выпустили из другой папки, не принимайте: выключите режим автора и обновитесь.
            </div>
          </div>
          <Button variant="primary" onClick={adopt}>
            <PackageCheck size={15} />
            Принять {published}
          </Button>
        </div>
      )}

      {revertedFrom && (
        <div className="panel px-5 py-4 text-[13px]">
          Папка возвращена к версии {installed}, игроки пока остаются на {revertedFrom}.
          <div className="mt-1 text-muted">
            Проверьте игру и выпустите новую версию ниже: она вернёт игроков к {installed}. Моды из неё возьмутся из старых
            релизов, загружать почти нечего. Передумали — верните папку к {revertedFrom} в разделе «Версии сборки».
          </div>
        </div>
      )}

      <Section
        title="Список модов"
        action={
          <Button variant="ghost" className="-mr-2" onClick={refresh}>
            <RotateCw size={14} />
            Обновить
          </Button>
        }
      >
        <div className="px-5 pb-4">
          {!pending ? (
            <span className="text-[13px] text-muted">Загрузка…</span>
          ) : listChanges.length === 0 ? (
            <span className="flex items-center gap-2 text-[13px] text-muted">
              <ListTree size={15} />
              Список модов совпадает с версией {installed}
            </span>
          ) : (
            <ul className="space-y-1.5">
              {listChanges.map(([label, text]) => (
                <li key={label} className="text-[13px]">
                  <span className="text-accent">{label}:</span> <span className="break-words">{text}</span>
                </li>
              ))}
            </ul>
          )}
          {pending && pending.personal.length > 0 && (
            <p className="mt-3 text-[13px] text-faint">
              Не войдут в выпуск, под разделителем LYNO USER MODS: {pending.personal.join(", ")}. Чтобы мод ушёл игрокам,
              перетащите его в MO2 выше этого разделителя.
            </p>
          )}
        </div>
      </Section>

      <Section title="Файлы модов">
        <div className="space-y-3 px-5 pb-4">
          <p className="text-[13px] text-muted">
            Лаунчер перечитает все моды сборки и покажет, какие файлы вы изменили. Это займёт столько же времени, сколько
            чтение всей сборки с диска. Новые моды здесь не проверяются: они и так войдут в выпуск целиком. Отменить правки
            мода можно в его меню на странице модов: «Вернуть опубликованную версию».
          </p>
          {verifyProgress ? (
            <UpdateProgress progress={verifyProgress} onCancel={api.cancelVerify} />
          ) : (
            <Button onClick={verify}>
              <FileSearch size={15} />
              Проверить файлы
            </Button>
          )}
          {verifyReport && verifyReport.damaged.length === 0 && verifyReport.customized.length === 0 && (
            <div className="flex items-center gap-2 text-[13px] text-muted">
              <Check size={15} className="text-ok" />
              Файлы модов не менялись
            </div>
          )}
          {verifyReport && (verifyReport.damaged.length > 0 || verifyReport.customized.length > 0) && (
            <ul className="divide-y divide-line/60 overflow-hidden rounded-xl bg-bg/60">
              {verifyReport.damaged.map((d) => {
                const settings = verifyReport.customized.find((c) => c.id === d.id);
                return (
                  <li key={d.id ?? ""} className="px-4 py-2.5">
                    <span className="block truncate text-sm">{titleOf(d.id, d.folder)}</span>
                    {describe(d).map((line) => (
                      <span key={line} className="block text-[13px] break-words text-muted">
                        {line}
                      </span>
                    ))}
                    {settings && (
                      <span className="block text-[13px] break-words text-muted">
                        Настройки уйдут игрокам: {list(settings.files)}
                      </span>
                    )}
                  </li>
                );
              })}
              {verifyReport.customized.filter((c) => !verifyReport.damaged.some((d) => d.id === c.id)).map((c) => (
                <li key={`settings-${c.id}`} className="px-4 py-2.5" title={c.files.sample.join("\n")}>
                  <span className="block truncate text-sm">{titleOf(c.id, c.folder)}</span>
                  <span className="block text-[13px] break-words text-muted">
                    Настройки уйдут игрокам: {list(c.files)}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </div>
      </Section>

      <Section title="Новая версия">
        <div className="px-5 pb-5">
          {/* A new installed version (accepted or just published) starts a fresh form: next version, no notes. */}
          <ReleaseBuilder key={installed} canPublish={canPublish} pending={pending} />
        </div>
      </Section>

      <Section title="Бета и релиз">
        <ChannelPromote canPublish={canPublish} />
      </Section>

      <Section title="Версии сборки">
        <div className="space-y-3 px-5 pb-5">
          <p className="text-[13px] text-muted">
            Если выпуск оказался плохим, верните папку к прошлой версии: лаунчер поставит её как обновление. Установленная
            версия {installed} отменяет ваши изменения с неё. Моды сборки встанут как в выбранной версии: её файлы, порядок,
            включённые и выключенные, удалённые вернутся. Моды, которых в ней нет, уйдут под LYNO USER MODS. Игроки ничего
            не заметят, пока вы не выпустите новую версию.
          </p>
          {!canPublish ? (
            <span className="text-[13px] text-muted">Нужен токен GitHub: история выпусков читается через него.</span>
          ) : progress ? (
            <UpdateProgress progress={progress} onCancel={api.cancelUpdate} />
          ) : !versions ? (
            <span className="text-[13px] text-muted">Загрузка…</span>
          ) : !revertBuild ? (
            <span className="text-[13px] text-muted">Опубликованных версий нет</span>
          ) : (
            <div className="flex flex-wrap items-center gap-3">
              <select
                value={revertBuild.commit}
                onChange={(e) => {
                  setRevertTo(e.target.value);
                  setConfirmRevert(false);
                }}
                className="h-9 rounded-full bg-raised px-4 font-mono text-[13px] outline-none focus:ring-1 focus:ring-neon/50"
              >
                {versions.builds.map((b) => (
                  <option key={b.commit} value={b.commit}>
                    {b.version}
                    {b.version === installed ? " (установлена)" : ""}
                    {b.version === revertedFrom ? " (опубликована)" : ""}
                    {b.beta ? " (бета)" : ""}
                  </option>
                ))}
              </select>
              {confirmRevert ? (
                <>
                  <div className="basis-full text-[13px] text-warn">
                    {edited.length > 0
                      ? `Изменённые моды получат файлы ${revertBuild.version}, правки в них пропадут: ${edited.join(", ")}.`
                      : `Правки в файлах лаунчер видит только после «Проверки файлов» выше. Без неё они останутся в модах, которые в ${revertBuild.version} те же, а в остальных пропадут.`}
                  </div>
                  <Button variant="primary" onClick={() => void startRevert(revertBuild.commit, revertBuild.beta)}>
                    <History size={15} />
                    Да, вернуть к {revertBuild.version}
                  </Button>
                  <Button variant="ghost" onClick={() => setConfirmRevert(false)}>
                    Отмена
                  </Button>
                </>
              ) : (
                <Button onClick={() => setConfirmRevert(true)}>
                  <History size={15} />
                  Вернуть папку к {revertBuild.version}
                </Button>
              )}
            </div>
          )}
        </div>
      </Section>

      <Section title="Доступ">
        <Credentials onChange={(s) => setCanPublish(s.github)} />
      </Section>
    </div>
  );
}

function listLines(p: Pending): [string, string][] {
  const lines: [string, string][] = [];
  if (p.added.length > 0) lines.push([`Новые (${p.added.length})`, p.added.join(", ")]);
  if (p.removed.length > 0) lines.push([`Удалены (${p.removed.length})`, p.removed.join(", ")]);
  if (p.renamed.length > 0) lines.push([`Переименованы (${p.renamed.length})`, p.renamed.map(([a, b]) => `${a} → ${b}`).join(", ")]);
  if (p.toggled.length > 0) lines.push([`Включены или выключены (${p.toggled.length})`, p.toggled.join(", ")]);
  if (p.separators.length > 0) lines.push([`Разделители (${p.separators.length})`, `новые, переименованы или другой цвет: ${p.separators.join(", ")}`]);
  if (p.metadata.length > 0) lines.push([`Метаданные и зависимости (${p.metadata.length})`, p.metadata.join(", ")]);
  if (p.reordered) lines.push(["Порядок", "изменён порядок модов или разделителей"]);
  return lines;
}

function describe(d: Damaged): string[] {
  switch (d.problem.kind) {
    case "missingFolder":
      return ["Папка мода удалена, а мод остался в списке"];
    case "changed":
      return ["Файлы изменены. Мод поставлен старой версией лаунчера, подробностей по файлам нет."];
    case "files": {
      const { missing, changed, added } = d.problem;
      const lines: string[] = [];
      if (changed.count > 0) lines.push(`Изменено ${changed.count} ${plural(changed.count, "файл", "файла", "файлов")}: ${list(changed)}`);
      if (added.count > 0) lines.push(`Добавлено ${added.count} ${plural(added.count, "файл", "файла", "файлов")}: ${list(added)}`);
      if (missing.count > 0) lines.push(`Удалено ${missing.count} ${plural(missing.count, "файл", "файла", "файлов")}: ${list(missing)}`);
      return lines;
    }
  }
}
