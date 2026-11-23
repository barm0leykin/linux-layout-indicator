//! Слежение за сменой активного окна через EWMH `_NET_ACTIVE_WINDOW`.
//!
//! Нужно для режима `show_on_focus`: когда пользователь переходит в другое
//! окно, мы проверяем (через AT-SPI), есть ли там поле ввода, и показываем
//! текущую раскладку. Это событийно — без опроса.

use std::sync::mpsc;

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{AtomEnum, ChangeWindowAttributesAux, ConnectionExt as _, EventMask};

/// Слушать смену активного окна. Сигнализирует в `tx` без значения.
pub fn watch(tx: mpsc::Sender<()>) -> Result<()> {
    let (conn, screen_num) = x11rb::connect(None).context("соединение для слежения за фокусом")?;
    let root = conn.setup().roots[screen_num].root;

    let active_atom = conn
        .intern_atom(false, b"_NET_ACTIVE_WINDOW")
        .context("intern _NET_ACTIVE_WINDOW")?
        .reply()
        .context("ответ intern")?
        .atom;

    conn.change_window_attributes(
        root,
        &ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE),
    )
    .context("подписка на PropertyNotify корня")?;
    conn.flush().context("flush")?;

    // Свойство может выставляться несколько раз за одно переключение —
    // сигналим только когда окно действительно сменилось.
    let mut last: Option<u32> = None;

    loop {
        let event = conn.wait_for_event().context("ожидание события X")?;
        if let Event::PropertyNotify(notify) = event {
            if notify.window != root || notify.atom != active_atom {
                continue;
            }
            let current = conn
                .get_property(false, root, active_atom, AtomEnum::WINDOW, 0, 1)
                .ok()
                .and_then(|cookie| cookie.reply().ok())
                .and_then(|reply| reply.value32().and_then(|mut it| it.next()));

            if current == last {
                continue;
            }
            last = current;

            if tx.send(()).is_err() {
                break;
            }
        }
    }
    Ok(())
}
