# Дизайн: layout-indicator

Документ описывает сущности проекта, их связи, модель потоков и применённые
паттерны проектирования. Дополняет [PRD.md](../PRD.md) (что нужно) и
[tasks.md](../tasks.md) (в каком порядке делаем).

---

## 1. Границы и назначение

Утилита-демон на Rust: слушает смену раскладки клавиатуры и показывает
небольшой бейдж рядом с точкой ввода. Не перехватывает ввод, не требует root,
работает локально.

Три независимые оси, которые архитектура разводит по слоям:

| Ось | Вопрос | Слой |
|---|---|---|
| **Источник раскладки** | откуда узнаём про смену языка | `backend/` |
| **Точка привязки** | где именно показывать бейдж | `resolve_anchor` (движки) |
| **Отрисовка** | как показать, не мешая другим | `renderer/` |

---

## 2. Слои

```
┌───────────────────────────────────────────────────────────────┐
│ main: главный цикл (анимация, подавление, координация)         │
│   Signal{ Layout | Focus | FieldFocus }  ← mpsc                │
└───────▲───────────────▲───────────────▲───────────────▲────────┘
        │               │               │               │
   backend-поток   focus-поток     click-поток     a11y-потоки
   (XKB | D-Bus)   (_NET_ACTIVE)   (XInput2 raw)   (AT-SPI)
        │               │               │               │
   ┌────┴────┐     X11 Property      XInput2        D-Bus AT-SPI
   │ X11 XKB │        Notify          raw events
   │ KDE DBus│
   └─────────┘

         main → Presenter (состояния) → render_badge → X11Overlay
         main → TrayState ←→ tray (StatusNotifierItem)
```

Слои общаются только через `main`; между собой — нет.

---

## 3. Сущности по модулям

### 3.1 `main.rs` — оркестрация

| Сущность | Ответственность |
|---|---|
| `Signal` | событие для главного цикла: `Layout(u8)`, `Focus`, `FieldFocus(Rect)` |
| `Runtime` | всё, что пересоздаётся при смене конфига: `cfg`, `font`, `overlay`, `config_mtime`. Метод `maybe_reload()` — hot reload |
| `BadgeContext<'a>` | «пакет» ссылок для показа: runtime, a11y, tray, codes, position, click, last_caret |
| `show_badge(...)` | политика показа: пауза, fullscreen, пересчёт позиции, передача в presenter |
| `resolve_anchor(...)` | **движки позиционирования** (см. §6.1) |
| `position_for(...)` | чистая функция: точка привязки → координаты окна с учётом монитора и краёв |
| `backend_thread(...)` | поток-производитель событий раскладки |
| `run/demo/watch/doctor/...` | режимы работы |

### 3.2 `backend/` — источник раскладки

| Сущность | Ответственность |
|---|---|
| `trait Backend` | контракт: `current_group()`, `next_change()` |
| `BackendKind` | `auto | kde | xkb` |
| `open(kind)` | **фабрика**: KDE D-Bus, иначе XKB |
| `layout_codes()` | коды раскладок: сначала KDE, иначе `kxkbrc`/`setxkbmap` |
| `X11Backend` | подписка на `XkbStateNotify` |
| `KdeBackend` | `org.kde.KeyboardLayouts`: `getLayout`, сигнал `layoutChanged` |

### 3.3 `presenter.rs` — политика показа

| Сущность | Ответственность |
|---|---|
| `Presenter` | тайминги, кросс-фейд, антидребезг текста |
| `Phase` | `Hidden → FadeIn → Hold → FadeOut` (конечный автомат) |
| `layers()` | какие слои текста рисовать (1 или 2 при кросс-фейде) |

### 3.4 `renderer/` — отрисовка

| Сущность | Ответственность |
|---|---|
| `Layer` | слой текста (текст + альфа) |
| `render_badge(...)` | **чистая функция**: appearance + шрифт + слои → `Pixmap` |
| `rounded_rect`, `draw_text`, `blend_pixel` | примитивы (tiny-skia + ab_glyph) |
| `X11Overlay` | ARGB-окно: `override_redirect`, пустая input-shape, `put_image` |

### 3.5 `a11y.rs` — каретка через AT-SPI

| Сущность | Ответственность |
|---|---|
| `Rect` | прямоугольник в координатах экрана |
| `A11y` | воркер запросов каретки (tokio-рантайм в отдельном потоке) |
| `spawn_focus_listener` | подписка на `object:state-changed:focused`, кэш найденного объекта |
| `caret_at`, `caret_of` | каретка у объекта (прямой `TextProxy`) |
| `caret_in_subtree` | ограниченный поиск по поддереву сфокусированного узла |
| `find_caret` | полный обход дерева (дорогой путь) |

### 3.6 Прочие модули

| Модуль | Сущности | Ответственность |
|---|---|---|
| `click.rs` | `ClickState`, `watch()` | XInput2 raw-события → точка последнего левого клика (TTL 3 с) |
| `focus.rs` | `watch()` | `_NET_ACTIVE_WINDOW` через PropertyNotify |
| `screen.rs` | `Monitor`, `monitors()`, `monitor_at()` | геометрия мониторов из `xrandr` |
| `layout.rs` | `layout_codes()`, `code_for()` | индекс группы → код раскладки |
| `theme.rs` | `Theme`, `palette()`, `resolve()` | системная тема и цвета по умолчанию |
| `config.rs` | `Config` и вложенные структуры | YAML, валидация, дефолты |
| `tray.rs` | `TrayState`, `LayoutTray` | StatusNotifierItem: пауза, конфиг, выход |
| `instance.rs` | `InstanceGuard` | один экземпляр на сессию (`flock`) |
| `log.rs` | `init/info/debug` | stderr → journald |

---

## 4. Потоки данных

### 4.1 Смена раскладки

```
XKB event / KDE layoutChanged
        │
   backend-поток → Signal::Layout(group)
        │
   main: codes[kind] → show_badge()
        │
   resolve_anchor() → position_for() → Presenter::show()
        │
   каждый кадр: render_badge() → X11Overlay::draw_at()
```

### 4.2 Фокус на текстовом поле (GTK/Electron)

```
AT-SPI event object:state-changed:focused(enabled)
        │
   a11y-listener: caret_at(объект) → Rect
        │
   Signal::FieldFocus(rect) → main: last_caret = rect; show_badge(anchor=rect)
```

### 4.3 Клик (простой движок)

```
XInput2 XI_RawButtonPress (левая)
        │
   click-поток: query_pointer → ClickState
        │
   при следующем показе: resolve_anchor() читает ClickState.recent()
```

---

## 5. Модель потоков

| Поток | Что делает | Как общается |
|---|---|---|
| главный | анимация, политика, отрисовка | `recv_timeout(frame_delay)` |
| backend | XKB-события или D-Bus-сигналы | `mpsc → Signal` |
| focus (X11) | PropertyNotify `_NET_ACTIVE_WINDOW` | `mpsc → Signal::Focus` |
| click (XInput2) | raw-нажатия кнопок | общий `Arc<ClickState>` |
| a11y-listener | события фокуса AT-SPI | `mpsc → Signal::FieldFocus` |
| a11y-воркер | запросы каретки по требованию | канал запрос/ответ |
| tray | сервис SNI + периодический refresh | общий `Arc<TrayState>` |

Главный цикл — единственный, кто трогает окно и presenter: остальные потоки
только производят события. Это убирает гонки за состояние UI.

---

## 6. Паттерны проектирования

### 6.1 Strategy — движки позиционирования

`Positioning` (`auto | pointer | click | caret | window`) — выбор стратегии
вычисления точки привязки. Реализация в `resolve_anchor()`: каждая ветка —
отдельный алгоритм с общей сигнатурой.

Цепочка `auto` — это **Chain of Responsibility**: каретка из событий → точка
клика → курсор мыши; первая сработавшая стратегия выигрывает.

### 6.2 Strategy + Factory — источники раскладки

`BackendKind` + `backend::open()` — фабрика, возвращающая `Box<dyn Backend>`.
`X11Backend` и `KdeBackend` — взаимозаменяемые стратегии получения раскладки.

### 6.3 Adapter — бэкенды

`X11Backend` адаптирует поток XKB-событий, `KdeBackend` — D-Bus-сигналы
`org.kde.KeyboardLayouts`; оба приводят источник к одному интерфейсу
`Backend`. `main` не знает, откуда пришла раскладка.

### 6.4 Dependency Inversion

`main` зависит от трейта `Backend`, а не от конкретных реализаций.
Добавление GNOME-бэкенда не потребует правок главного цикла.

### 6.5 State — конечный автомат показа

`Presenter` с фазами `Hidden → FadeIn → Hold → FadeOut`. Переходы зависят
только от времени и фазы, что делает политику детерминированной и тестируемой
без X-сервера.

### 6.6 Observer / event-driven

Все внешние источники — событийные, без поллинга:
`XkbStateNotify`, PropertyNotify, XInput2 raw events, D-Bus-сигналы, AT-SPI
events. Фоновый поллинг есть только как аварийный режим (`backend.polling_ms`).

### 6.7 Producer–Consumer

Потоки-производители (`backend`, `focus`, `a11y`) отправляют `Signal` в канал,
потребитель — главный цикл. `recv_timeout(frame_delay)` одновременно задаёт
темп анимации и тайм-аут ожидания событий.

### 6.8 Facade

- `Runtime` — фасад над «окружением» (конфиг + шрифт + окно), скрывает их
  пересоздание при hot reload.
- `X11Overlay` — фасад над X11-протоколом (окно, GC, shape, put_image).

### 6.9 Guard / RAII

- `InstanceGuard` — блокировка `flock` снимается при выходе процесса;
  устаревших lock-файлов не бывает.
- `X11Overlay.mapped` — состояние окна, чтобы не вызывать `unmap` повторно.

### 6.10 Value Objects

`Color`, `Rect`, `Monitor`, `Layer`, `LayoutChanged` — неизменяемые значения
с методами проверки/преобразования. Упрощают тесты и убирают «сырые» кортежи
из публичных интерфейсов.

### 6.11 Pure functions

`render_badge`, `position_for`, `code_for`, `palette` — без побочных эффектов;
покрыты юнит-тестами (включая рендер пикселей) без запуска X-сервера.

### 6.12 Graceful Degradation / Null Object

Отказ подсистемы не ломает продукт:
- нет композитора → непрозрачный бейдж;
- нет a11y → откат на клик/мышь;
- нет StatusNotifierWatcher → работа без трея;
- нет XKB-событий → режим поллинга.

### 6.13 Anti-Corruption Layer

`a11y.rs` изолирует «сырой» AT-SPI (zbus, объектные ссылки, интерфейсы) от
внутренней модели: наружу торчит только `Rect`. Все особенности AT-SPI
(Qt без `GetInterfaces`, нулевые размеры, зависшие приложения) остаются внутри.

---

## 7. Ключевые архитектурные решения

Полный список — в [tasks.md §11 «Журнал решений»](../tasks.md). Кратко:

| Решение | Почему |
|---|---|
| Ядро + сменные бэкенды/движки | Wayland и GNOME требуют принципиально разных механизмов |
| KDE D-Bus вместо KWin-эффекта | `org.kde.KeyboardLayouts` работает и в X11, и в Wayland, не требует плагина |
| Отрисовка на CPU в RGBA-буфер | один `PutImage`, полный контроль над альфой, тестируемо |
| Событийность вместо поллинга | нулевая нагрузка в простое |
| AT-SPI как опция, не основа | системная настройка с рисками (см. инцидент 2026-11-24) |

---

## 8. Как расширять

**Новый источник раскладки** (например, GNOME Wayland):
1. реализовать `Backend` в `backend/gnome.rs`;
2. добавить вариант в `BackendKind`;
3. добавить ветку в `backend::open()`;
4. при необходимости — коды раскладок в `backend::layout_codes()`.

**Новый движок позиционирования**:
1. добавить вариант в `Positioning`;
2. добавить ветку в `resolve_anchor()`;
3. обновить README (таблица движков) и `config.example.yaml`.

**Другая платформа (Windows/macOS)**: тот же `Backend` + отдельный рендерер
окна; presenter и конфиг переиспользуются как есть.

---

## 9. Известные ограничения архитектуры

- Рендерер привязан к X11 (`X11Overlay`): для Wayland нужен отдельный
  `LayerShellOverlay` либо отрисовка силами композитора.
- Каретка доступна не везде: Qt требует полного обхода (дорого и рискованно),
  терминалы не отдают её вовсе.
- Главный цикл однопоточный: тяжёлый запрос каретки блокирует анимацию на
  время тайм-аута.
