#!/usr/bin/env python3
"""Тестовое GTK3-окно с полем ввода для проверки каретки через AT-SPI.

Запускается уже после включения accessibility, поэтому GTK строит полное
a11y-дерево. Ничего не меняет в системе.

run: python3 -I gtk_entry_test.py [seconds]
"""
import sys

import gi

gi.require_version("Gtk", "3.0")
from gi.repository import GLib, Gtk  # noqa: E402

WINDOW_TITLE = "a11y caret test"


def main():
    seconds = int(sys.argv[1]) if len(sys.argv) > 1 else 60

    win = Gtk.Window(title=WINDOW_TITLE)
    win.set_default_size(420, 120)
    win.set_position(Gtk.WindowPosition.CENTER)

    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL, spacing=8)
    box.set_border_width(12)
    entry = Gtk.Entry()
    entry.set_text("проверка каретки")
    box.pack_start(entry, False, False, 0)
    win.add(box)

    win.connect("destroy", Gtk.main_quit)
    win.show_all()

    def focus():
        win.present()
        entry.grab_focus()
        entry.set_position(len(entry.get_text()))
        return False

    GLib.timeout_add(400, focus)
    GLib.timeout_add(seconds * 1000, Gtk.main_quit)
    Gtk.main()


if __name__ == "__main__":
    main()
