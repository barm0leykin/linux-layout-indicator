# layout-indicator

Всплывающий индикатор текущей раскладки клавиатуры рядом с курсором мыши
при переключении языка. Работает на X11; Wayland — отдельный этап (см. PRD).

**Статус:** рабочий MVP для X11 (M1). Демон следит за раскладкой, показывает
бейдж «EN»/«RU» с плавным появлением и исчезновением.

## Документация

- [PRD.md](PRD.md) — требования, архитектура, риски
- [tasks.md](tasks.md) — задачи по этапам
- [docs/spike-report.md](docs/spike-report.md) — результаты технической разведки

## Требования

- Linux с X11 и **композитором** (KWin, picom, …) — без него не будет прозрачности
- Раскладки, переключаемые на уровне XKB (штатное переключение KDE/GNOME)
- Для шрифта по умолчанию: `fonts-dejavu-core`

## Сборка и запуск

```sh
cargo build --release
./target/release/layout-indicator
```

Полезные режимы:

| Команда | Что делает |
|---|---|
| `layout-indicator` | запустить демон |
| `layout-indicator --demo EN` | показать бейдж один раз (проверка внешнего вида) |
| `layout-indicator --watch` | печатать каждое переключение раскладки |
| `layout-indicator --print-config` | напечатать YAML с настройками по умолчанию |
| `layout-indicator --help` | справка |

## Конфигурация

Файл: `~/.config/layout-indicator/config.yaml` (или `$XDG_CONFIG_HOME/layout-indicator/config.yaml`).
Если файла нет — используются значения по умолчанию. Любой ключ можно опустить.

Полный пример со всеми значениями: [config.example.yaml](config.example.yaml).

```yaml
appearance:
  width: 88
  height: 44
  corner_radius: 8.0
  font_path: /usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf
  font_size: 20.0
  text_color: '#ffffff'
  background_color: '#1b1c1d'
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
```

Изменения в файле применяются при следующем запуске (hot reload — в работе).

## Автозапуск

```sh
mkdir -p ~/.config/autostart
cat > ~/.config/autostart/layout-indicator.desktop <<'EOF'
[Desktop Entry]
Type=Application
Name=Layout Indicator
Exec=/путь/до/layout-indicator
X-GNOME-Autostart-enabled=true
EOF
```

## Структура

```
src/
  backend/    источники события смены раскладки (X11)
  presenter/  политика показа (тайминги, фазы)
  renderer/   отрисовка бейджа и X11-окно
  config.rs   конфигурация (YAML)
  layout.rs   XKB-группа → код раскладки
spike/        одноразовые прототипы M0 (C/Python)
docs/         отчёты
```

## Приватность

Утилита работает локально: нет сетевых запросов, нет логирования нажатий
и содержимого полей. Подробнее — PRD §9.

## Известные ограничения

- Только X11. В Wayland сторонние приложения не могут наблюдать за раскладкой —
  нужен компонент внутри композитора (этап M3).
- Позиционирование — у курсора мыши. Показ у текстовой каретки требует включённой
  accessibility в сессии и перезапуска приложений (см. `docs/spike-report.md`, M0-05).
- Ограничение по краям считается по всему десктопу, а не по отдельному монитору.

## Лицензия

MIT, см. [LICENSE](LICENSE).
