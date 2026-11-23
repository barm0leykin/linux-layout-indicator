//! X11-бэкенд: событийное отслеживание смены раскладки.
//!
//! Механика проверена в спайке (`spike/xkb_watch.c`): подписка на
//! `XkbStateNotify` с маской `GROUP_STATE` даёт ровно одно событие на
//! реальное переключение, без перехвата клавиш и без root.

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xkb::{self, ConnectionExt as _, EventType, ID, MapPart, StatePart};
use x11rb::rust_connection::RustConnection;

use super::Backend;

/// Подключение к X-серверу с подпиской на смену группы раскладки.
pub struct X11Backend {
    conn: RustConnection,
}

impl X11Backend {
    /// Подключиться к дисплею из `$DISPLAY` и подписаться на смену группы.
    pub fn new() -> Result<Self> {
        let (conn, _screen_num) =
            x11rb::connect(None).context("не удалось подключиться к X-серверу")?;

        // Расширение XKB должно поддерживаться сервером.
        conn.xkb_use_extension(1, 0)
            .context("запрос xkb_use_extension")?
            .reply()
            .context("XKB-расширение недоступно на этом X-сервере")?;

        // Подписка только на смену состояния группы (не на модификаторы).
        let details = xkb::SelectEventsAux {
            state_notify: Some(xkb::SelectEventsAuxStateNotify {
                affect_state: StatePart::GROUP_STATE,
                state_details: StatePart::GROUP_STATE,
            }),
            ..Default::default()
        };

        conn.xkb_select_events(
            ID::USE_CORE_KBD.into(),
            EventType::from(0u16), // clear
            EventType::from(0u16), // select_all
            MapPart::from(0u16),   // affect_map
            MapPart::from(0u16),   // map
            &details,
        )
        .context("подписка на XkbStateNotify")?;

        conn.flush().context("flush")?;

        Ok(Self { conn })
    }
}

impl Backend for X11Backend {
    fn current_group(&self) -> Result<u8> {
        let reply = self
            .conn
            .xkb_get_state(ID::USE_CORE_KBD.into())
            .context("запрос xkb_get_state")?
            .reply()
            .context("ответ xkb_get_state")?;
        Ok(reply.group.into())
    }

    fn next_change(&mut self) -> Result<u8> {
        loop {
            let event = self.conn.wait_for_event().context("ожидание события X")?;
            if let Event::XkbStateNotify(notify) = event {
                return Ok(notify.group.into());
            }
        }
    }
}
