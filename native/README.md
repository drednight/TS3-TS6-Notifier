# tsn-native — нативное ядро (в разработке)

Полностью нативная замена Electron-сервера на Rust (без внешних крейтов):
коннекторы TeamSpeak, модель состояния и HTTP/WS-сервер для того же веб-оверлея.

**Статус (этап 6 из 6 — финальная регрессия):**
- ✅ ядро: конфиг `config.json` (совместим с Electron-версией), модель состояния
- ✅ TS3: плагин-мост (Overwolf-style, TCP 48912) + ClientQuery-фолбэк
- ✅ TS6: Remote Apps (WS-клиент, auth + API key, все события, порт ts6.ts 1:1)
- ✅ HTTP + WS: тот же протокол (`hello`/`state`/`config`/`move-*`), статика `public/`
- ✅ «Диагностика» (`/api/diagnostics`), move-mode API
- ✅ нативный оверлей: свой рендерер (SDF-AA, шрифты DejaVu-атлас, иконки классов),
  прозрачное окно поверх всех (win32 layered), drag + clamp, move-mode ✓/✗,
  обводка говорящего, шёпот с задержкой, лента событий, 8 тем и **12 стилей
  оформления** (6 спокойных + 6 анимированных: неон, тактический, плазма,
  сияние, матрица, радар); выбор стиля — живые миниатюры с группами
- ✅ источник «Авто»: проба TS3/TS6, живой TS6 важнее пустого TS3-клиента,
  переключение после 8 с простоя (фактический источник виден в трее)
- ✅ TS6: heartbeat (ping каждые 10 с) и watchdog (45 с без входящих кадров) —
  «мёртвый» линк переподключается сам, оверлей не застревает в «live»
- ✅ окно настроек «Концепт 3» (превью-стек): пресеты/цвета/слайдеры/тумблеры,
  закреплённый превью справа, живое применение, открывается один раз из шестерни
- ✅ системный трей: иконка, тултип статуса (ru/en), «Источник данных» (синхронно
  с веб-панелью), «Настройки», «Выход»
- ✅ UAC: `desktop.elevate` (по умолчанию вкл.) → перезапуск с правами
  администратора; отказ от UAC продолжает работу без повышения
- ✅ установка: zip-пакет + Inno Setup (`native/installer/tsn-native.iss`),
  сборка Windows-бинарника и пакета в CI (artifacts)

## Сборка и запуск

```bash
cd native && cargo build --release
TSN_ROOT=.. PORT=48911 ./target/release/tsn-native
```

`TSN_ROOT` — каталог с `public/` и `data/` (без него — каталог exe, если рядом
есть `public/`, иначе текущий). OBS-URL и веб-настройки работают как в
Electron-сборке.

## Windows-пакет и установщик

CI (`ci-native`, job `windows`) собирает `tsn-native.exe` и выкладывает
артефакты **tsn-native-win64** (zip: exe + `public/` + `data/` + плагин) и
**tsn-native-setup** (Inno Setup). Локально:

```powershell
cargo build --release --manifest-path native/Cargo.toml
pwsh native/tools/package-win.ps1              # dist-win/tsn-native-win64.zip
iscc native/installer/tsn-native.iss           # dist-win/TS3-TS6-Notifier-Setup.exe
```

Релизы не публикуются (по договорённости) — только артефакты CI.

## Тесты

```bash
cd native && cargo test
```
