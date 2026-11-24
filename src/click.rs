//! «Простой» движок позиционирования: точка последнего клика.
//!
//! Клик по текстовому полю — это почти всегда место, где окажется каретка.
//! Слушаем сырые события XInput2 (`XI_RawButtonPress`): они приходят на весь
//! сервер, не перехватывают ввод и не требуют accessibility. Позицию указателя
//! на момент нажатия забираем отдельным запросом.

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::Event;
use x11rb::protocol::xinput::{ConnectionExt as _, EventMask, XIEventMask};
use x11rb::protocol::xproto::ConnectionExt as _;

/// Насколько «свежим» должен быть клик, чтобы его использовать.
const CLICK_TTL: Duration = Duration::from_secs(3);

/// Последний левый клик.
pub struct ClickState {
    last: Mutex<Option<((i16, i16), Instant)>>,
}

impl ClickState {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            last: Mutex::new(None),
        })
    }

    fn set(&self, position: (i16, i16)) {
        if let Ok(mut guard) = self.last.lock() {
            *guard = Some((position, Instant::now()));
        }
    }

    /// Позиция последнего клика, если он был недавно.
    pub fn recent(&self) -> Option<(i16, i16)> {
        let guard = self.last.lock().ok()?;
        let (position, at) = (*guard)?;
        (at.elapsed() <= CLICK_TTL).then_some(position)
    }
}

impl Default for ClickState {
    fn default() -> Self {
        Self {
            last: Mutex::new(None),
        }
    }
}

/// Слушать нажатия кнопок мыши. Блокирующий цикл — вызывать в отдельном потоке.
pub fn watch(state: Arc<ClickState>) -> Result<()> {
    let (conn, screen_num) = x11rb::connect(None).context("соединение для кликов")?;
    let root = conn.setup().roots[screen_num].root;

    conn.xinput_xi_query_version(2, 1)
        .context("запрос версии XInput2")?
        .reply()
        .context("XInput2 недоступен")?;

    // deviceid = 1 — все master-устройства (XIAllMasterDevices).
    let mask = EventMask {
        deviceid: 1,
        mask: vec![XIEventMask::RAW_BUTTON_PRESS],
    };
    conn.xinput_xi_select_events(root, &[mask])
        .context("подписка на сырые нажатия")?;
    conn.flush().context("flush")?;

    loop {
        let event = conn.wait_for_event().context("ожидание события X")?;
        if let Event::XinputRawButtonPress(press) = event {
            // 1 — левая кнопка.
            if press.detail == 1 {
                let reply = conn
                    .query_pointer(root)
                    .context("query_pointer")?
                    .reply()
                    .context("ответ query_pointer")?;
                state.set((reply.root_x, reply.root_y));
            }
        }
    }
}
