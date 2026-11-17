# Плагин TS3 для TS3-TS6 Notifier

Плагин для **клиента TeamSpeak 3** (`.dll`), который стримит события (входы,
выходы, перемещения, говорение, чат, поки, группы) прямо в приложение
TS3-TS6 Notifier по локальному порту `127.0.0.1:48912` — как в Overwolf
для TeamSpeak. События идут из самого клиента: без ClientQuery, без
поллинга, без задержек и обрывов соединения.

## Установка

1. Скачайте `ts3plugin_tsn_win64.dll` из релиза (или соберите — ниже).
2. Скопируйте в папку плагинов клиента:
   `%APPDATA%\TS3Client\plugins\`
   (в приложении: **«Ещё» → «Плагин TS3» → «Установить плагин»** — сделает это автоматически).
3. Перезапустите TeamSpeak 3 и включите плагин
   **Settings → Options → Add-ons** → «TS3-TS6 Notifier bridge» → **Enable**.

Пока плагин работает, приложение использует его вместо ClientQuery
(автоматически; ClientQuery остаётся резервом, если плагин не установлен).

## Сборка

Из x64 Developer Prompt Visual Studio:

```
cd plugin
build.bat
```

Заголовки SDK клиента TeamSpeak 3 лежат в `include/` (официальный
[ts3client-pluginsdk](https://github.com/teamspeak/ts3client-pluginsdk)).

## Протокол

Строки JSON через `\n`, плагин → приложение: `hello`, `snapshot`
(полный список клиентов), `join`, `leave`, `move`, `talk`, `chat`,
`poke`, `client` (обновление имени/групп), `disconnect`. Приложение →
плагин: `{"cmd":"snapshot"}` и `{"cmd":"ping"}`.
