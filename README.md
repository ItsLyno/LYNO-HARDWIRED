# LYNO//HARDWIRED

Лаунчер сборки модов Cyberpunk 2077 поверх Mod Organizer 2. Лаунчер не заменяет MO2: он управляет портативным инстансом MO2 (моды, `meta.ini`, `modlist.txt`) и запускает игру через его командную строку. Моды скачиваются с Nexus Mods аккаунтом пользователя, ничего не перезаливается.

## Структура

| Путь | Что это |
|---|---|
| `crates/core` | `lyno-core`: парсинг `modlist.txt` / `meta.ini`, манифест сборки, план обновления |
| `crates/pack` | `lyno-pack`: утилита автора сборки, экспорт инстанса MO2 в манифест |
| `apps/launcher` | Tauri 2 + React + TypeScript + Tailwind |

## Разработка

```sh
cargo test -p lyno-core -p lyno-pack     # ядро
cd apps/launcher && pnpm install
pnpm dev                                 # UI в браузере на мок-данных
pnpm tauri dev                           # приложение целиком (Windows)
```

Черновик манифеста из своего инстанса MO2:

```sh
cargo run -p lyno-pack -- scan "D:\Modding\MO2" --profile LYNO --out manifest.json
```

Установщик для Windows собирает GitHub Actions (`.github/workflows/ci.yml`, артефакт `lyno-hardwired-setup`).

## Статус

- [x] Этап 1: каркас, ядро, запуск игры и MO2, UI-шелл
- [ ] Этап 2: установка MO2, поиск игры, настройка инстанса
- [ ] Этап 3: рецепты установки в `lyno-pack`
- [ ] Этап 4: загрузка с Nexus и обновление сборки
- [ ] Этап 5: проверки и разбор логов
- [ ] Этап 6: полировка, автообновление лаунчера
