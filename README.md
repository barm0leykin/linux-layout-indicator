# layout-indicator

Всплывающий индикатор текущей раскладки клавиатуры рядом с курсором мыши
при переключении языка. Работает на X11; Wayland — отдельный этап (см. PRD).

**Статус:** MVP для X11 завершён (M1). Демон следит за раскладкой, показывает
бейдж «EN»/«RU» с плавным появлением, кросс-фейдом при быстром переключении
и исчезновением.

## Документация

- [PRD.md](PRD.md) — требования, архитектура, риски
- [tasks.md](tasks.md) — задачи по этапам
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
  hot_reload: true
backend:
  polling_ms: null       # например 200 — опрос вместо XKB-событий
```

`theme: auto` определяет тему по `~/.config/kdeglobals` (KDE); если определить
не удалось — используется тёмная палитра.

## Позиция каретки (AT-SPI)

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

## Показ при смене окна

`behavior.show_on_focus: true` — показывать раскладку при переходе в другое окно,
если в нём есть поле ввода. Полезно вместе с `SwitchMode=WinClass` в KDE, когда
раскладка переключается по приложению и пользователь не видит, какая она теперь.

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

- Только X11. В Wayland сторонние приложения не могут наблюдать за раскладкой —
  нужен компонент внутри композитора (этап M3).
- Позиционирование по умолчанию — у курсора мыши. Режим `caret` требует включённой
  accessibility в сессии и перезапуска приложений (см. `docs/spike-report.md`, M0-05).
- `show_on_focus` срабатывает на смену **окна**, а не отдельного поля ввода:
  дальше проверяется, есть ли в окне текстовое поле.
- Если выбран `positioning: caret`, а accessibility выключена, каждый показ ждёт
  ответа AT-SPI до 300 мс и затем откатывается к курсору мыши — бейдж появится
  с задержкой. По умолчанию используется `pointer`.
- Скругления и прозрачность требуют композитора; без него бейдж непрозрачный.
- Смена раскладки по приложению (`SwitchMode=WinClass` в KDE) обрабатывается
  как обычное переключение.

## Лицензия

MIT, см. [LICENSE](LICENSE).
