import { Check, ExternalLink, KeyRound, Link2, Loader2, LogOut } from "lucide-react";
import { useEffect, useState } from "react";
import { api } from "../api";
import { useApp } from "../store";
import { Button } from "./Button";
import { Section } from "./Section";

const API_KEYS_URL = "https://next.nexusmods.com/settings/api-keys";

// Account and nxm links; downloads live in the footer.
export function NexusAccount() {
  const { nexus, refreshNexus } = useApp();
  useEffect(() => {
    void refreshNexus();
  }, []);
  if (!nexus) return <p className="text-[13px] text-muted">Загрузка…</p>;

  return (
    <div className="space-y-3">
      <Account />
      {nexus.handler.supported && <Handler />}
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
