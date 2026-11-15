/* Minimal i18n: ru/en dictionaries, auto-detection, DOM attribute binding. */
(function () {
  const RU = {
    // overlay
    noChannel: 'Нет подключения',
    emptyChannel: 'В канале никого нет',
    emptyChannelSub: 'Дождитесь, пока кто-нибудь зайдёт на канал',
    you: 'вы',
    statusConnecting: 'Подключение…',
    statusLive: 'Подключено',
    statusWaitingAuth: 'Нужно разрешение',
    statusOffline: 'Не подключено',
    statusError: 'Ошибка',
    sourceMock: 'Демо-режим',
    sourceTs3: 'TeamSpeak 3',
    sourceTs6: 'TeamSpeak 6',
    hintMock: 'Демо-данные: оверлей оживает без запущенного TeamSpeak',
    hintTs3: 'Ожидание ClientQuery…',
    hintTs6: 'Ожидание Remote Apps…',
    // drawer
    settingsSubtitle: 'Настройки оверлея',
    tabAppearance: 'Вид',
    tabElements: 'Элементы',
    tabConnection: 'Связь',
    tabAbout: 'Ещё',
    theme: 'Тема',
    themeGlass: 'Стекло',
    themeMidnight: 'Полночь',
    themeFrost: 'Иней',
    themeNeon: 'Неон',
    accentColor: 'Акцентный цвет',
    glow: 'Свечение',
    opacity: 'Прозрачность панели',
    blur: 'Размытие фона',
    radius: 'Скругление',
    scale: 'Масштаб',
    fontSize: 'Размер шрифта',
    corner: 'Положение на экране',
    margin: 'Отступ от края',
    language: 'Язык',
    langAuto: 'Как в системе',
    eqStyle: 'Индикатор речи',
    eqBars: 'Столбики',
    eqRing: 'Кольцо',
    eqDots: 'Точки',
    eqNone: 'Скрыть',
    showHeader: 'Заголовок канала',
    showServerName: 'Имя сервера',
    showUserCount: 'Счётчик участников',
    showBanner: 'Баннер состояния',
    showAvatars: 'Аватары',
    showBadges: 'Значки статусов',
    showSelf: 'Показывать себя',
    hideQuery: 'Скрывать query-ботов',
    talkingFirst: 'Говорящие — сверху',
    onlyTalking: 'Только говорящие',
    compact: 'Компактный режим',
    hideEmpty: 'Скрывать пустую панель',
    animations: 'Анимации',
    dataSource: 'Источник данных',
    dataSourceHint: 'TS3 — плагин ClientQuery; TS6 — Remote Apps; демо работает без клиента',
    ts3KeyHint: 'TS3: Настройки → Плагины → Client Query → Settings… → API key',
    ts6KeyHint: 'TS6: ключ выдаётся после подтверждения запроса в Настройки → Remote Apps',
    reconnect: 'Переподключить',
    connectionHelp:
      'TS3: включите плагин ClientQuery и введите API key. TS6: включите Remote Apps (порт 5899) и разрешите приложение при первом подключении.',
    aboutText:
      'Красивый оверлей для TeamSpeak 3 и TeamSpeak 6: состав канала, индикация речи, статусы и гибкая настройка внешнего вида. Данные берутся только из локального клиента.',
    aboutTip:
      'Совет: добавьте страницу как Browser Source в OBS (прозрачный фон) или откройте в окне браузера поверх игры. Параметры URL (?corner=br&opacity=0.5&accent=22d3ee&clean=1) переопределяют настройки — удобно для готовых сцен.',
    profile: 'Профиль настроек',
    exportBtn: 'Экспорт',
    importBtn: 'Импорт',
    resetBtn: 'Сбросить',
  };

  const EN = {
    noChannel: 'Not connected',
    emptyChannel: 'Nobody in this channel',
    emptyChannelSub: 'Waiting for someone to join',
    you: 'you',
    statusConnecting: 'Connecting…',
    statusLive: 'Connected',
    statusWaitingAuth: 'Permission needed',
    statusOffline: 'Not connected',
    statusError: 'Error',
    sourceMock: 'Demo mode',
    sourceTs3: 'TeamSpeak 3',
    sourceTs6: 'TeamSpeak 6',
    hintMock: 'Demo data: the overlay is alive without TeamSpeak',
    hintTs3: 'Waiting for ClientQuery…',
    hintTs6: 'Waiting for Remote Apps…',
    settingsSubtitle: 'Overlay settings',
    tabAppearance: 'Look',
    tabElements: 'Parts',
    tabConnection: 'Link',
    tabAbout: 'More',
    theme: 'Theme',
    themeGlass: 'Glass',
    themeMidnight: 'Midnight',
    themeFrost: 'Frost',
    themeNeon: 'Neon',
    accentColor: 'Accent color',
    glow: 'Glow',
    opacity: 'Panel opacity',
    blur: 'Background blur',
    radius: 'Corner radius',
    scale: 'Scale',
    fontSize: 'Font size',
    corner: 'Screen corner',
    margin: 'Edge margin',
    language: 'Language',
    langAuto: 'System',
    eqStyle: 'Speaking indicator',
    eqBars: 'Bars',
    eqRing: 'Ring',
    eqDots: 'Dots',
    eqNone: 'Hidden',
    showHeader: 'Channel header',
    showServerName: 'Server name',
    showUserCount: 'User counter',
    showBanner: 'Status banner',
    showAvatars: 'Avatars',
    showBadges: 'Status badges',
    showSelf: 'Show yourself',
    hideQuery: 'Hide query bots',
    talkingFirst: 'Talking users first',
    onlyTalking: 'Only talking users',
    compact: 'Compact mode',
    hideEmpty: 'Hide empty panel',
    animations: 'Animations',
    dataSource: 'Data source',
    dataSourceHint: 'TS3 — ClientQuery plugin; TS6 — Remote Apps; demo works without a client',
    ts3KeyHint: 'TS3: Settings → Plugins → Client Query → Settings… → API key',
    ts6KeyHint: 'TS6: the key is issued after you approve the request in Settings → Remote Apps',
    reconnect: 'Reconnect',
    connectionHelp:
      'TS3: enable the ClientQuery plugin and paste the API key. TS6: enable Remote Apps (port 5899) and allow the app on first connect.',
    aboutText:
      'A beautiful overlay for TeamSpeak 3 and TeamSpeak 6: channel roster, speaking indicators, statuses and deep appearance tuning. Data comes only from your local client.',
    aboutTip:
      'Tip: add the page as an OBS Browser Source (transparent background) or open it in a browser window over your game. URL params (?corner=br&opacity=0.5&accent=22d3ee&clean=1) override settings — perfect for prepared scenes.',
    profile: 'Settings profile',
    exportBtn: 'Export',
    importBtn: 'Import',
    resetBtn: 'Reset',
  };

  const dict = { ru: RU, en: EN };

  function detect(preference) {
    if (preference === 'ru' || preference === 'en') return preference;
    const nav = (navigator.language || 'en').toLowerCase();
    return nav.startsWith('ru') ? 'ru' : 'en';
  }

  let lang = detect('auto');

  function t(key) {
    return dict[lang][key] ?? dict.en[key] ?? key;
  }

  function applyDom() {
    document.querySelectorAll('[data-i18n]').forEach((el) => {
      el.textContent = t(el.getAttribute('data-i18n'));
    });
    document.documentElement.lang = lang;
  }

  function setLanguage(preference) {
    lang = detect(preference);
    applyDom();
    window.dispatchEvent(new CustomEvent('tsn:lang', { detail: lang }));
  }

  window.I18N = { t, setLanguage, applyDom, get lang() { return lang; } };
})();
