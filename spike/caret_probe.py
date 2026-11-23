#!/usr/bin/env python3
"""M0-05: проверка доступности позиции текстовой каретки через AT-SPI2.

Особенности реального окружения (выяснено в спайке):
  * STATE_ACTIVE может стоять сразу на нескольких окнах разных приложений;
  * сами окна STATE_FOCUSED не имеют — флаг только у вложенных элементов;
  * дерево бывает большим (Chrome/VS Code) — нужен бюджет обхода.

run:  python3 -I caret_probe.py            # один снимок
      python3 -I caret_probe.py --watch 60 # следить за сменой фокуса
"""
import sys
import time

import pyatspi

NODE_BUDGET = 4000      # максимум узлов на одно окно
TIME_BUDGET = 2.0       # секунд на одно окно


def get_state(obj):
    try:
        return obj.getState()
    except Exception:
        return None


def has(obj, flag):
    st = get_state(obj)
    return st is not None and st.contains(flag)


def active_windows():
    """Все окна со STATE_ACTIVE: [(app_name, window), ...]."""
    out = []
    desktop = pyatspi.Registry.getDesktop(0)
    for app in desktop:
        try:
            app_name = app.name or "?"
        except Exception:
            app_name = "?"
        try:
            for win in app:
                if has(win, pyatspi.STATE_ACTIVE):
                    out.append((app_name, win))
        except Exception:
            continue
    return out


def role_name(obj):
    try:
        return obj.getRoleName()
    except Exception:
        return "?"


def obj_name(obj):
    try:
        return obj.name or ""
    except Exception:
        return ""


def caret_of(obj):
    """Возвращает dict с данными о каретке или строку-причину отказа."""
    try:
        text = obj.queryText()
    except Exception:
        return None
    try:
        offset = text.caretOffset
    except Exception as e:
        return f"caretOffset error: {e}"
    try:
        ext = text.getCharacterExtents(offset, pyatspi.DESKTOP_COORDS)
    except Exception as e:
        return f"extents error: {e}"
    return {"offset": offset, "rect": tuple(ext)}


def find_focused_text(root):
    """BFS по дереву: первый элемент с STATE_FOCUSED и работающим Text."""
    queue = [(root, 0)]
    visited = 0
    deadline = time.time() + TIME_BUDGET
    while queue and visited < NODE_BUDGET and time.time() < deadline:
        node, depth = queue.pop(0)
        visited += 1
        if depth > 0 and has(node, pyatspi.STATE_FOCUSED):
            info = caret_of(node)
            if isinstance(info, dict):
                return node, info
        if depth >= 40:
            continue
        try:
            n = node.childCount
        except Exception:
            continue
        for i in range(n):
            try:
                queue.append((node.getChildAtIndex(i), depth + 1))
            except Exception:
                continue
    return None, None


def probe():
    """Один снимок: список результатов по всем активным окнам."""
    t_start = time.time()
    results = []
    wins = active_windows()
    if not wins:
        return ["нет активных окон"]

    for app_name, win in wins:
        t0 = time.time()
        node, info = find_focused_text(win)
        dt = (time.time() - t0) * 1000
        if node is None:
            results.append(f"[{app_name}] win={obj_name(win)!r} -> фокус с Text не найден ({dt:.0f} мс)")
            continue
        r = info["rect"]
        results.append(
            f"[{app_name}] role={role_name(node)} name={obj_name(node)!r} "
            f"-> caret={info['offset']} rect=({r[0]},{r[1]},{r[2]}x{r[3]}) ({dt:.0f} мс)"
        )
    results.append(f"-- всего {(time.time() - t_start) * 1000:.0f} мс")
    return results


def main():
    watch = 0
    if "--watch" in sys.argv:
        i = sys.argv.index("--watch")
        if i + 1 < len(sys.argv):
            watch = int(sys.argv[i + 1])

    if not watch:
        for line in probe():
            print(line)
        return

    print(f"наблюдение {watch} s — переводите фокус в разные приложения/поля")
    sys.stdout.flush()
    t0 = time.time()
    last = None
    while time.time() - t0 < watch:
        try:
            res = "\n".join(probe())
        except Exception as e:
            res = f"ошибка: {e}"
        if res != last:
            print(f"--- {time.time() - t0:.1f}s ---")
            print(res)
            sys.stdout.flush()
            last = res
        time.sleep(1.0)
    print("done")


if __name__ == "__main__":
    main()
