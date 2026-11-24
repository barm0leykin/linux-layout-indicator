# layout-indicator

Всплывающий индикатор текущей раскладки клавиатуры рядом с точкой ввода
при переключении языка. Работает на X11 и **KDE Wayland**; GNOME Wayland — в планах.

**Статус:** MVP для X11 завершён (M1–M2, M4). KDE Wayland поддержан: раскладка
читается через `org.kde.KeyboardLayouts`, бейдж рисуется через Xwayland.

## Документация

- [PRD.md](PRD.md) — требования, архитектура, риски
- [tasks.md](tasks.md) — задачи по этапам
- [docs/design.md](docs/design.md) — **сущности, слои и паттерны проектирования**
- [docs/spike-report.md](docs/spike-report.md) — результаты технической разведки

## Требования

- Linux с X11 и **композитором** (KWin, picom, …) — без него бейдж рисуется
  непрозрачным (деградация автоматическая)
- Раскладки, переключаемые на уровне XKB (штатное переключение KDE/GNOME)
- Для шрифта по умолчанию: `fonts-dejavu-core`

## Сборка и запуск

```sh
cargo build --release
./target/release/layout-indicator
```

| Команда | Что делает |
|---|---|
| `layout-indicator` | запустить демон (второй экземпляр не запустится) |
| `layout-indicator --demo EN` | показать бейдж один раз |
| `layout-indicator --watch` | печатать каждое переключение раскладки |
| `layout-indicator --doctor` | диагностика окружения |
| `layout-indicator --caret-probe` | проверить, доступна ли позиция каретки |
| `layout-indicator --print-config` | напечатать YAML с настройками по умолчанию |
| `layout-indicator --install-autostart` | добавить автозапуск в `~/.config/autostart` |
| `layout-indicator -v` | подробный лог в stderr (journald) |
| `layout-indicator --help` | справка |

## Конфигурация

Файл: `~/.config/layout-indicator/config.yaml` (или `$XDG_CONFIG_HOME/layout-indicator/config.yaml`).
Если файла нет — используются значения по умолчанию. Любой ключ можно опустить.
Файл перечитывается на лету (`behavior.hot_reload`).

Полный пример: [config.example.yaml](config.example.yaml).

```yaml
appearance:
  width: 88
  height: 44
  corner_radius: 8.0
  font_path: /usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf
  font_size: 20.0
  theme: auto            # auto | dark | light
  text_color: null       # null — из палитры темы, иначе '#ffffff'
  background_color: null # null — из палитры темы
  border_color: null     # null — из палитры темы (рамка для контраста)
  border_width: 1.0      # px, 0 — без рамки
  background_alpha: 0.88
timing:
  fade_in_ms: 120
  hold_ms: 700
  fade_out_ms: 200
offset:
  dx: 14
  dy: -24
behavior:
  suppress_fullscreen: true
  show_on_focus: false
  focus_requires_text_field: false
  deep_search: true      # полный обход дерева a11y (Qt; первый раз медленно)
  hot_reload: true
backend:
  kind: auto             # auto | kde | xkb
  polling_ms: null       # например 200 — опрос вместо событий
```

### Источники раскладки

| `backend.kind` | Механизм | Где работает |
|---|---|---|
| `auto` (по умолчанию) | KDE D-Bus, иначе XKB | везде, выбор автоматический |
| `kde` | `org.kde.KeyboardLayouts` (`getLayout`, сигнал `layoutChanged`) | KDE X11 **и** KDE Wayland |
| `xkb` | `XkbStateNotify` | прочие X11 (i3, Xfce) |

`theme: auto` определяет тему по `~/.config/kdeglobals` (KDE); если определить
не удалось — используется тёмная палитра.

## Движки позиционирования

`positioning` выбирает, где показывать бейдж. Движки сменные — точка привязки
вычисляется в одном месте (`resolve_anchor`), каждый включается конфигом:

| Значение | Движок | Нужна a11y | Комментарий |
|---|---|---|---|
| `auto` (по умолчанию) | каретка из событий → точка клика → мышь | нет | безопасная цепочка: каретка берётся только из уже пришедших AT-SPI-событий, без обхода дерева |
| `click` | точка последнего левого клика (XInput2) | нет | приблизительно, зато работает в любом приложении — включая Qt и терминалы |
| `caret` | запрос каретки через AT-SPI | **да** | точнее, но может быть дорогим и небезопасным на некоторых сборках Qt (см. предупреждение ниже) |
| `window` | угол активного окна | нет | грубо, но предсказуемо |
| `pointer` | курсор мыши | нет | работает всегда |

Для GNOME (X11) пригодны `auto`, `click`, `pointer`, `window`. Для GNOME Wayland
понадобится отдельный движок (Shell extension) — этап M3.

## Позиция каретки (AT-SPI)

> ⚠️ **Внимание.** Включение accessibility — системная настройка, влияющая на все
> приложения. На проверенной системе (Ubuntu 22.04, KDE Plasma 5.24, Qt 5.15)
> после включения `IsEnabled=true` **Qt-приложения падали с SIGSEGV**
> (Kate, Notepadqq; отчёты в `/var/crash`). Включайте осознанно и выключайте,
> если приложения начали падать:
>
> ```sh
> gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
>   --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled "<false>"
> ```
>
> Утилита **сама accessibility не включает** — только использует, если она уже включена.

`positioning: caret` — бейдж появляется у текстовой каретки, а не у мыши.

Требования:

- в сессии включён `org.a11y.Status.IsEnabled`;
- приложение запущено **после** включения (иначе оно не строит дерево доступности);
- для Electron/Chromium может понадобиться `--force-renderer-accessibility`.

Включить accessibility в текущей сессии:

```sh
gdbus call --session --dest org.a11y.Bus --object-path /org/a11y/bus \
  --method org.freedesktop.DBus.Properties.Set org.a11y.Status IsEnabled "<true>"
```

Проверить: `layout-indicator --caret-probe`.

Если каретку получить не удалось, происходит автоматический откат к курсору мыши.
Позиция каретки запрашивается только в момент показа — в простое AT-SPI не опрашивается.

`behavior.deep_search` (по умолчанию `true`) — если быстрый путь не дал каретку,
выполняется полный обход дерева доступности. Нужен для Qt-приложений; первый раз
может занять секунды, затем найденный объект кэшируется по приложению.

**Статус совместимости:** GTK-приложения (gedit, Firefox, Telegram) и Electron —
каретка работает. Qt (Kate, Notepadqq) — каретка зависит от полного обхода; если
приложение не отвечает по a11y-шине, используется фолбэк на мышь. Терминалы
каретку не отдают никогда — там всегда фолбэк.

## Показ при смене окна и фокусе поля

`behavior.show_on_focus: true` — показывать раскладку при переходе в другое окно.
Триггер — смена активного окна (`_NET_ACTIVE_WINDOW`), работает **без** accessibility.

Если accessibility включена, дополнительно срабатывает **фокус на текстовом поле** —
в том числе клик по полю внутри уже активного окна (подписка на
`object:state-changed:focused` через AT-SPI). Перед показом проверяется, что у
элемента есть каретка, поэтому клики по кнопкам и меню бейдж не вызывают.

`behavior.focus_requires_text_field: true` — строгий режим и для смены окна:
бейдж появится только если в окне есть текстовое поле (требует accessibility).

При `positioning: caret` бейдж встаёт у каретки, иначе — по настройке позиционирования.

## Трей

Демон регистрирует иконку через StatusNotifierItem (D-Bus).

- **Левый клик** — пауза / включение индикатора.
- **Правый клик** — меню: приостановить, открыть конфиг, выход.
- В тултипе — текущая раскладка или пометка «Пауза».
- Иконка — чёрный квадрат с белым глифом из `img/ico.png`: вшита в бинарник
  в размерах 22/24/32/48 px, отдельных файлов при установке не требуется.

Если StatusNotifierWatcher недоступен, демон работает без иконки —
это не ошибка, а деградация.

## Автозапуск

```sh
./target/release/layout-indicator --install-autostart
```

Либо вручную — пример файла в [packaging/layout-indicator.desktop](packaging/layout-indicator.desktop).

## Пакет .deb

```sh
packaging/build-deb.sh
```

Собирает `target/debian/layout-indicator_<версия>_<арх>.deb` — только `cargo` и `dpkg-deb`,
никаких дополнительных инструментов. Установка: `sudo apt install ./target/debian/*.deb`.

В `Cargo.toml` также есть метаданные для `cargo-deb`, если он у вас установлен.

Для Arch — заготовка в [packaging/PKGBUILD](packaging/PKGBUILD) (требует публичного репозитория).

## Структура

```
src/
  backend/    источники события смены раскладки (X11)
  presenter/  политика показа (тайминги, фазы, кросс-фейд)
  renderer/   отрисовка бейджа и X11-окно
  config.rs   конфигурация (YAML)
  layout.rs   XKB-группа → код раскладки
  screen.rs   геометрия мониторов (xrandr)
  theme.rs    системная тема и палитра
  instance.rs защита от второго экземпляра
  log.rs      логирование в stderr
spike/        одноразовые прототипы M0 (C/Python)
packaging/    .desktop и прочее для упаковки
docs/         отчёты
```

## Приватность

Утилита работает локально: нет сетевых запросов, нет логирования нажатий
и содержимого полей. Подробнее — PRD §9.

- При `positioning: caret` через AT-SPI читаются только координаты каретки —
  содержимое полей не запрашивается.
- При `show_on_focus` проверяется лишь факт, что в окне есть текстовое поле.
- В логах (`-v`) — только индексы групп, координаты и идентификаторы окон.

## Известные ограничения

- **GNOME Wayland** не поддержан: нужен отдельный бэкенд (Shell extension или
  монитор dconf/gsettings). KDE Wayland работает — см. раздел «Источники раскладки».
- Под KDE Wayland бейдж рисуется через Xwayland, поэтому полноэкранное
  подавление и позиционирование могут вести себя иначе, чем на X11.
- Позиционирование по умолчанию — у курсора мыши. Режим `caret` требует включённой
  accessibility в сессии и перезапуска приложений (см. `docs/spike-report.md`, M0-05).
- Без включённой accessibility `show_on_focus` срабатывает только на смену окна;
  клик по полю внутри активного окна бейдж не вызовет.
- Если выбран `positioning: caret`, а accessibility выключена, каждый показ ждёт
  ответа AT-SPI до 300 мс и затем откатывается к курсору мыши — бейдж появится
  с задержкой. По умолчанию используется `pointer`.
- Скругления и прозрачность требуют композитора; без него бейдж непрозрачный.
- Смена раскладки по приложению (`SwitchMode=WinClass` в KDE) обрабатывается
  как обычное переключение.

## Лицензия

MIT, см. [LICENSE](LICENSE).
