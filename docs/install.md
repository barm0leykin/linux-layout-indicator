# Установка

## Требования

| Что | Зачем |
|---|---|
| Linux с X11 | поддерживаемая платформа (Wayland — этап M3) |
| Композитор (KWin, picom, …) | прозрачность и скругления; без него бейдж непрозрачный |
| Раскладки через XKB | штатное переключение KDE/GNOME |
| `fonts-dejavu-core` | шрифт бейджа по умолчанию |
| Rust 1.85+ и gcc | только для сборки из исходников |
| KDE Plasma / любая оболочка с StatusNotifierItem | для иконки в трее (необязательно) |

## Вариант 1. Пакет .deb (Ubuntu / Debian)

```sh
git clone https://github.com/barm0leykin/linux-layout-indicator.git
cd linux-layout-indicator
packaging/build-deb.sh                      # соберёт target/debian/*.deb
sudo apt install ./target/debian/layout-indicator_0.1.0_amd64.deb
```

Бинарник встанет в `/usr/bin/layout-indicator`, ярлык — в меню приложений.
Пакет не запускает демон сам: автозапуск включается отдельно (см. ниже).

## Вариант 2. Из исходников

```sh
git clone https://github.com/barm0leykin/linux-layout-indicator.git
cd linux-layout-indicator
cargo build --release
```

Запуск без установки:

```sh
./target/release/layout-indicator
```

Установка в систему:

```sh
sudo install -Dm755 target/release/layout-indicator /usr/local/bin/layout-indicator
sudo install -Dm644 packaging/layout-indicator.desktop \
    /usr/share/applications/layout-indicator.desktop
```

## Вариант 3. Arch Linux

Пока пакета в AUR нет: нужен публичный репозиторий, чтобы заполнить `source=`
в `packaging/PKGBUILD`. До тех пор — сборка из исходников (вариант 2).

## Автозапуск

```sh
layout-indicator --install-autostart
```

Создаёт `~/.config/autostart/layout-indicator.desktop` — демон будет
запускаться при входе в сессию. Команду нужно выполнять **от своего
пользователя**, не через `sudo`.

Проверить, что демон поднялся:

```sh
pgrep -f target/release/layout-indicator   # или /usr/bin/layout-indicator
```

## Настройка

```sh
mkdir -p ~/.config/layout-indicator
cp config.example.yaml ~/.config/layout-indicator/config.yaml
```

Либо в трее: правый клик → «Открыть конфиг» (файл создастся с настройками
по умолчанию, если его ещё нет).

Файл перечитывается на лету — перезапуск не нужен.

Основное:

```yaml
positioning: pointer   # pointer | caret | window
appearance:
  width: 88
  height: 44
  font_size: 20.0
  theme: auto          # auto | dark | light
timing:
  hold_ms: 700
behavior:
  suppress_fullscreen: true
  show_on_focus: false
```

## Проверка после установки

```sh
layout-indicator --doctor      # окружение, раскладки, композитор, тема
layout-indicator --demo RU     # показать бейдж один раз
layout-indicator --watch       # печатать переключения раскладки
```

Затем нажмите `Alt+Shift` — бейдж должен появиться рядом с курсором.

## Использование трея

- **Левый клик** по иконке — пауза / включение индикатора.
- **Правый клик** — меню: приостановить, открыть конфиг, выход.
- В тултипе видна текущая раскладка или пометка «Пауза».

Если трея в системе нет (или нет StatusNotifierWatcher), демон работает
как обычно — просто без иконки.

## Удаление

```sh
# остановить: правый клик по иконке → «Выход»
# или
pkill -f layout-indicator

rm -f ~/.config/autostart/layout-indicator.desktop
sudo apt remove layout-indicator          # если ставили пакетом
rm -rf ~/.config/layout-indicator         # настройки (по желанию)
```

## Устранение неполадок

| Симптом | Причина | Что делать |
|---|---|---|
| Бейдж не появляется | демон не запущен | `pgrep -f layout-indicator`, перезапустить |
| Бейдж без прозрачности | нет композитора | включить compositing в KDE; это ожидаемое поведение |
| Бейдж не виден в полноэкранном приложении | так задумано | `behavior.suppress_fullscreen: false` |
| «уже запущен» | работает второй экземпляр | это защита; закройте старый |
| Каретка недоступна | a11y выключена или приложение не перезапущено | см. README, раздел «Позиция каретки» |
| Нет иконки в трее | нет StatusNotifierWatcher | демон работает и без неё |
| Бейдж слишком большой/маленький | размеры в конфиге | `appearance.width/height/font_size` |

Полная диагностика: `layout-indicator --doctor`.
