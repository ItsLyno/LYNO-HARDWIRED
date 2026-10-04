import { ArrowRight, Check, Download, ExternalLink, KeyRound, Link2, Loader2, LogOut, RefreshCw, X } from "lucide-react";
import { useEffect, useState } from "react";
import { api, type ModUpdateRow, type NexusJob, type Outcome } from "../api";
import { Button } from "../components/Button";
import { PageTitle } from "../components/PageTitle";
import { Section } from "../components/Section";
import { formatBytes, plural } from "../format";
import { isJobActive, useApp } from "../store";

const API_KEYS_URL = "https://next.nexusmods.com/settings/api-keys";

export function Nexus() {
  const { nexus, nexusUpdates, nexusJobs, refreshNexus } = useApp();
  useEffect(() => {
    void refreshNexus();
  }, []);
  if (!nexus) return <p className="text-muted">Загрузка…</p>;

  return (
    <div className="mx-auto max-w-5xl space-y-6">
      <PageTitle sub="Обновления модов и загрузки с сайта Nexus Mods">Nexus</PageTitle>
      <Account />
      {nexus.handler.supported && <Handler />}
      {nexusJobs.length > 0 && <Jobs jobs={nexusJobs} />}
      {nexus.account && nexusUpdates && <Updates />}
    </div>
  );
}

function Account() {
  const { nexus, run, refreshNexus } = useApp();
  const [key, setKey] = useState("");
  const [busy, setBusy] = useState<"key" | "sso" | null>(null);
  if (!nexus) return null;

  const login = (how: "key" | "sso") =>
    run(async () => {
      setBusy(how);
      try {
        const status = how === "key" ? await api.nexusSetKey(key) : await api.nexusSsoLogin();
        useApp.setState({ nexus: status });
        setKey("");
        await refreshNexus();
      } finally {
        setBusy(null);
      }
    });
  const logout = () =>
    run(async () => {
      await api.nexusLogout();
      await refreshNexus();
    });

  if (nexus.account) {
    return (
      <Section
        title="Аккаунт"
        action={
          <Button variant="ghost" className="-mr-2 h-8 text-[13px]" onClick={logout}>
            <LogOut size={14} />
            Выйти
          </Button>
        }
      >
        <div className="px-5 pb-5">
          <div className="flex items-center gap-2 text-sm">
            <Check size={15} className="text-ok" />
            {nexus.account.name}
            <span className={`rounded-full px-2 py-0.5 text-[11px] ${nexus.account.premium ? "bg-accent/15 text-accent" : "bg-raised text-muted"}`}>
              {nexus.account.premium ? "Premium" : "бесплатный"}
            </span>
          </div>
          <p className="mt-1.5 text-[13px] text-muted">
            {nexus.account.premium
              ? "Обновления скачиваются прямо из лаунчера."
              : "Nexus отдаёт файлы бесплатным аккаунтам только по кнопке «Mod Manager Download» на сайте: лаунчер откроет нужную страницу, а ссылка с неё придёт сюда."}
          </p>
        </div>
      </Section>
    );
  }

  return (
    <Section title="Аккаунт">
      <div className="px-5 pb-5">
        <p className="text-[13px] text-muted">
          Нужен для проверки обновлений и скачивания модов. Ключ API — на сайте Nexus в настройках аккаунта, раздел API Keys,
          внизу страницы «Personal API Key». Ключ хранится в диспетчере учётных данных Windows.
        </p>
        {nexus.accountError && <p className="mt-2 text-[13px] text-bad">{nexus.accountError}</p>}
        <div className="mt-3 flex gap-2">
          <div className="relative min-w-0 flex-1">
            <KeyRound size={14} className="pointer-events-none absolute top-1/2 left-3.5 -translate-y-1/2 text-faint" />
            <input
              type="password"
              value={key}
              spellCheck={false}
              autoComplete="off"
              placeholder="Ключ API"
              onChange={(e) => setKey(e.target.value)}
              className="h-9 w-full rounded-full bg-raised pr-4 pl-9 font-mono text-[13px] outline-none focus:ring-1 focus:ring-neon/50"
            />
          </div>
          <Button variant="primary" onClick={() => login("key")} disabled={!!busy || !key.trim()}>
            {busy === "key" && <Loader2 size={15} className="animate-spin" />}
            Войти
          </Button>
          <Button variant="ghost" onClick={() => run(() => api.openUrl(API_KEYS_URL))}>
            Где взять ключ
            <ExternalLink size={13} />
          </Button>
        </div>
        {nexus.sso && (
          <div className="mt-3 flex items-center gap-3">
            <Button onClick={() => login("sso")} disabled={!!busy}>
              {busy === "sso" && <Loader2 size={15} className="animate-spin" />}
              Войти через сайт Nexus
            </Button>
            {busy === "sso" && (
              <>
                <span className="text-[13px] text-muted">Подтвердите вход в открывшемся браузере…</span>
                <Button variant="ghost" className="h-8 text-[13px]" onClick={() => void api.nexusSsoCancel()}>
                  Отменить
                </Button>
              </>
            )}
          </div>
        )}
      </div>
    </Section>
  );
}

function Handler() {
  const { nexus, run } = useApp();
  if (!nexus) return null;
  const { ours, other } = nexus.handler;
  const change = (register: boolean) =>
    run(async () => {
      const handler = register ? await api.nxmRegister() : await api.nxmUnregister();
      useApp.setState({ nexus: { ...nexus, handler } });
    });

  return (
    <Section title="Кнопка «Mod Manager Download»">
      <div className="flex items-start justify-between gap-6 px-5 pb-5">
        <div className="text-[13px] text-muted">
          {ours ? (
            <>
              <span className="text-ok">Ссылки nxm:// открывает лаунчер.</span> Нажмите «Mod Manager Download» у файла на Nexus — мод
              скачается и установится сюда. Архивы с выбором опций (FOMOD) и RAR лаунчер положит в загрузки MO2.
            </>
          ) : (
            <>
              Сейчас ссылки nxm:// открывает {other ? <span className="font-mono text-fg">{other}</span> : "другая программа или никто"}.
              Лаунчер может забрать их себе, а потом вернуть обратно. Если MO2 или Vortex снова заберут ссылки, нажмите кнопку ещё раз.
            </>
          )}
        </div>
        {ours ? (
          <Button onClick={() => change(false)}>Вернуть прежней программе</Button>
        ) : (
          <Button variant="primary" onClick={() => change(true)}>
            <Link2 size={15} />
            Открывать в лаунчере
          </Button>
        )}
      </div>
    </Section>
  );
}

const mo2Reasons: Record<string, string> = {
  fomod: "в архиве установщик с выбором опций (FOMOD)",
  format: "архив RAR или другой формат, который лаунчер не распаковывает",
  layout: "лаунчер не узнал структуру архива",
};

function outcomeText(o: Outcome): string {
  switch (o.kind) {
    case "installed":
      return `Установлен: «${o.folder}»`;
    case "mo2":
      return `Лежит в загрузках MO2: ${mo2Reasons[o.reason]}. Установите его в MO2 на вкладке «Загрузки».`;
    case "mo2Open":
      return "MO2 открыт, поэтому архив лежит в его загрузках: установите его там.";
  }
}

function Jobs({ jobs }: { jobs: NexusJob[] }) {
  const { run, refreshNexus } = useApp();
  const finished = jobs.some((j) => !isJobActive(j));
  return (
    <Section
      title="Загрузки"
      action={
        finished && (
          <Button
            variant="ghost"
            className="-mr-2 h-8 text-[13px]"
            onClick={() =>
              run(async () => {
                await api.nexusClearJobs();
                await refreshNexus();
              })
            }
          >
            Очистить завершённые
          </Button>
        )
      }
    >
      <ul className="divide-y divide-line/60 border-t border-line/60">
        {jobs.map((j) => (
          <JobRow key={j.id} job={j} />
        ))}
      </ul>
    </Section>
  );
}

function JobRow({ job }: { job: NexusJob }) {
  const { run } = useApp();
  const s = job.state;
  const title = job.title ?? (job.modId ? `Мод ${job.modId}` : "Ссылка nxm://");
  const pct = s.kind === "downloading" && s.total > 0 ? (s.done / s.total) * 100 : null;
  let line: React.ReactNode;
  switch (s.kind) {
    case "queued":
      line = "В очереди";
      break;
    case "downloading":
      line = `${formatBytes(s.done)} / ${formatBytes(s.total)}`;
      break;
    case "retry":
      line = <span className="text-warn">Нет соединения, переподключение (попытка {s.attempt})…</span>;
      break;
    case "waiting":
      line = "Ждёт окончания обновления сборки или проверки файлов";
      break;
    case "installing":
      line = "Установка…";
      break;
    case "done":
      line = <span className={s.outcome.kind === "installed" ? "text-ok" : "text-warn"}>{outcomeText(s.outcome)}</span>;
      break;
    case "failed":
      line = <span className="text-bad">{s.error}</span>;
      break;
    case "cancelled":
      line = "Отменено";
      break;
  }
  const showMo2 = s.kind === "done" && s.outcome.kind !== "installed";

  return (
    <li className="px-5 py-3.5">
      <div className="flex items-center gap-3">
        <div className="min-w-0 flex-1">
          <div className="truncate text-sm">
            {title}
            {job.version && <span className="ml-2 font-mono text-xs text-muted">{job.version}</span>}
            {job.fileTitle && <span className="ml-2 text-[13px] text-faint">{job.fileTitle}</span>}
          </div>
          {job.replaces && isJobActive(job) && <div className="text-xs text-faint">заменит «{job.replaces}»</div>}
        </div>
        {showMo2 && (
          <Button className="h-8 text-[13px]" onClick={() => run(api.openMo2)}>
            Открыть MO2
          </Button>
        )}
        {isJobActive(job) && (
          <button
            onClick={() => run(() => api.nexusCancelJob(job.id))}
            title="Отменить"
            className="inline-flex size-7 items-center justify-center rounded-full text-muted hover:bg-raised hover:text-fg"
          >
            <X size={14} />
          </button>
        )}
      </div>
      {pct !== null && (
        <div className="mt-2 h-1 rounded-full bg-raised">
          <div className="h-full rounded-full bg-gradient-to-r from-neon to-accent transition-[width]" style={{ width: `${pct}%` }} />
        </div>
      )}
      <div className="mt-1 text-[13px] text-muted">{line}</div>
    </li>
  );
}

function ago(secs: number): string {
  const m = Math.round((Date.now() / 1000 - secs) / 60);
  if (m < 1) return "только что";
  if (m < 60) return `${m} ${plural(m, "минуту", "минуты", "минут")} назад`;
  const h = Math.round(m / 60);
  if (h < 48) return `${h} ${plural(h, "час", "часа", "часов")} назад`;
  const d = Math.round(h / 24);
  return `${d} ${plural(d, "день", "дня", "дней")} назад`;
}

const statusOrder: Record<string, number> = { update: 0, unavailable: 1, unknown: 2, upToDate: 3 };

function Updates() {
  const { nexus, nexusUpdates: view, nexusChecking, checkNexus, settings, run } = useApp();
  const [all, setAll] = useState(false);
  if (!view || !nexus?.account) return null;
  const author = !!settings?.authorMode;
  // A player's build mods come with the build: their Nexus versions are the author's business.
  const mods = view.mods.filter((m) => author || m.personal);
  const updates = mods.filter((m) => m.status.kind === "update");
  const shown = (all ? mods : mods.filter((m) => m.status.kind !== "upToDate")).sort(
    (a, b) => statusOrder[a.status.kind] - statusOrder[b.status.kind],
  );
  const premium = nexus.account.premium;

  const update = (m: ModUpdateRow) =>
    run(async () => {
      if (m.status.kind === "update" && m.status.file && premium) {
        await api.nexusDownload(m.game, m.modId, m.status.file.fileId);
      } else {
        await api.openUrl(m.pageUrl);
      }
    });

  return (
    <Section
      title="Обновления модов"
      action={
        nexusChecking ? (
          <span className="flex items-center gap-2 font-mono text-xs text-muted tabular-nums">
            <Loader2 size={14} className="animate-spin" />
            {nexusChecking.total > 0 ? `${nexusChecking.done} / ${nexusChecking.total}` : "Проверка…"}
            <Button variant="ghost" className="h-7 px-3 font-sans text-[13px]" onClick={() => void api.nexusCheckCancel()}>
              Отменить
            </Button>
          </span>
        ) : (
          <Button className="-mr-2 h-8 text-[13px]" onClick={() => void checkNexus(false)}>
            <RefreshCw size={14} />
            Проверить
          </Button>
        )
      }
    >
      <div className="flex flex-wrap items-center gap-x-4 gap-y-1 px-5 pb-3 text-[13px] text-muted">
        <span>
          {view.checkedAt ? `Проверено ${ago(view.checkedAt)}` : "Ещё не проверялось"} ·{" "}
          {updates.length > 0 ? (
            <span className="text-accent">
              {updates.length} {plural(updates.length, "обновление", "обновления", "обновлений")}
            </span>
          ) : (
            "обновлений нет"
          )}
        </span>
        {!author && <span className="text-faint">Моды сборки обновляются вместе со сборкой: здесь только ваши моды.</span>}
        <button onClick={() => setAll(!all)} className="ml-auto text-faint underline-offset-2 hover:text-muted hover:underline">
          {all ? "Только требующие внимания" : `Показать все (${mods.length})`}
        </button>
      </div>
      {shown.length > 0 ? (
        <table className="w-full table-fixed border-t border-line/60 text-left">
          <colgroup>
            <col />
            <col className="w-56" />
            <col className="w-40" />
          </colgroup>
          <tbody>
            {shown.map((m) => (
              <tr key={m.folder} className="h-12 border-b border-line/60 last:border-b-0">
                <td className="truncate pl-5 text-sm">
                  {m.folder}
                  {author && <span className="ml-2 rounded-full bg-raised px-2 py-0.5 text-[11px] text-muted">{m.personal ? "личный" : "сборка"}</span>}
                </td>
                <td className="truncate font-mono text-xs text-muted tabular-nums">
                  <StatusCell m={m} />
                </td>
                <td className="pr-5 text-right">
                  {m.status.kind === "update" && m.canUpdate && (
                    <Button
                      variant={premium && m.status.file ? "primary" : "secondary"}
                      className="h-8 text-[13px]"
                      title={premium && m.status.file ? undefined : "На странице нажмите «Mod Manager Download» у нужного файла"}
                      onClick={() => update(m)}
                    >
                      {premium && m.status.file ? <Download size={14} /> : <ExternalLink size={14} />}
                      {premium && m.status.file ? "Обновить" : "На Nexus"}
                    </Button>
                  )}
                  {m.status.kind !== "update" && (
                    <button
                      onClick={() => run(() => api.openUrl(m.pageUrl))}
                      className="inline-flex h-7 items-center gap-1.5 rounded-full px-2.5 text-[13px] text-muted hover:bg-raised hover:text-fg"
                    >
                      Nexus
                      <ExternalLink size={13} />
                    </button>
                  )}
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      ) : (
        <p className="border-t border-line/60 px-5 py-6 text-center text-[13px] text-muted">
          {mods.length === 0 ? "Нет модов со страницей на Nexus" : "Все моды актуальны"}
        </p>
      )}
      <div className="flex flex-wrap gap-x-4 border-t border-line/60 px-5 py-3 text-xs text-faint">
        {view.untracked.length > 0 && (
          <span title={view.untracked.join(", ")}>
            Без страницы Nexus в meta.ini: {view.untracked.length} {plural(view.untracked.length, "мод", "мода", "модов")} — их версии не отслеживаются
          </span>
        )}
        {view.rateLimit?.daily != null && <span className="ml-auto font-mono tabular-nums">Запросов к API осталось: {view.rateLimit.daily}</span>}
      </div>
    </Section>
  );
}

function StatusCell({ m }: { m: ModUpdateRow }) {
  const s = m.status;
  switch (s.kind) {
    case "update":
      return (
        <span className="inline-flex items-center gap-1.5">
          {m.version ?? "?"}
          <ArrowRight size={12} />
          <span className="text-accent">{s.version ?? "новая версия"}</span>
        </span>
      );
    case "upToDate":
      return <>{m.version ?? "—"}</>;
    case "unavailable":
      return <span className="font-sans text-warn">страница скрыта или удалена</span>;
    case "unknown":
      return <span className="font-sans text-faint">не проверялся</span>;
  }
}
