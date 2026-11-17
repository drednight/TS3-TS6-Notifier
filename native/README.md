# tsn-native — нативное ядро (в разработке)

Полностью нативная замена Electron-сервера на Rust (без внешних крейтов):
коннекторы TeamSpeak, модель состояния и HTTP/WS-сервер для того же веб-оверлея.

**Статус (этап 1 из 5):**
- ✅ ядро: конфиг `config.json` (совместим с Electron-версией), модель состояния
- ✅ TS3: плагин-мост (Overwolf-style, TCP 48912) + ClientQuery-фолбэк
- ✅ HTTP + WS: тот же протокол (`hello`/`state`/`config`/`move-*`), статика `public/`
- ✅ «Диагностика» (`/api/diagnostics`), установка плагина, move-mode API
- ⏳ TS6 (Remote Apps) — дальше; нативные окна оверлея/настроек; трей; обновления

## Сборка и запуск

```bash
cd native && cargo build --release
TSN_ROOT=.. PORT=48911 ./target/release/tsn-native
```

`TSN_ROOT` — каталог с `public/` и `data/` (по умолчанию текущий).
OBS-URL и веб-настройки работают как в Electron-сборке.

## Тесты

```bash
cd native && cargo test
```
