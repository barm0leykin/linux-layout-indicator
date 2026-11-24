//! KDE-бэкенд: раскладка через D-Bus `org.kde.KeyboardLayouts`.
//!
//! Работает и на X11, и на Wayland — этим и ценен: под Plasma Wayland
//! Xwayland раскладку не отдаёт, а этот сервис отдаёт.
//!
//! Интерфейс (`/Layouts`, `org.kde.KeyboardLayouts`):
//! * `getLayout() -> u` — текущий индекс;
//! * `getLayoutsList() -> a(sss)` — `(короткий код, вариант, имя)`;
//! * сигнал `layoutChanged(u)` — при переключении.

use anyhow::{Context, Result, bail};
use zbus::blocking::proxy::SignalIterator;
use zbus::blocking::{Connection, Proxy};

use super::Backend;

const SERVICE: &str = "org.kde.keyboard";
const PATH: &str = "/Layouts";
const INTERFACE: &str = "org.kde.KeyboardLayouts";

pub struct KdeBackend {
    connection: Connection,
}

impl KdeBackend {
    /// Подключиться к сессионной шине и убедиться, что сервис отвечает.
    pub fn new() -> Result<Self> {
        let connection = Connection::session().context("нет сессионной шины D-Bus")?;
        let backend = Self { connection };
        // Пробный вызов: если сервиса нет, вернём ошибку и вызывающий
        // откатится на XKB.
        backend
            .current_group()
            .context("org.kde.keyboard недоступен")?;
        Ok(backend)
    }

    fn proxy(&self) -> Result<Proxy<'_>> {
        Proxy::new(&self.connection, SERVICE, PATH, INTERFACE)
            .context("прокси org.kde.KeyboardLayouts")
    }
}

impl Backend for KdeBackend {
    fn current_group(&self) -> Result<u8> {
        let proxy = self.proxy()?;
        let index: u32 = proxy.call("getLayout", &()).context("getLayout")?;
        Ok(index.min(u32::from(u8::MAX)) as u8)
    }

    fn next_change(&mut self) -> Result<u8> {
        let proxy = self.proxy()?;
        let mut signals: SignalIterator<'_> = proxy
            .receive_signal("layoutChanged")
            .context("подписка на layoutChanged")?;

        for message in &mut signals {
            if let Ok(index) = message.body().deserialize::<u32>() {
                return Ok(index.min(u32::from(u8::MAX)) as u8);
            }
        }
        bail!("поток сигналов layoutChanged завершился")
    }
}

/// Коды раскладок из KDE (`us`, `ru`, …). `None` — сервис недоступен.
pub fn layout_codes() -> Option<Vec<String>> {
    let connection = Connection::session().ok()?;
    let proxy = Proxy::new(&connection, SERVICE, PATH, INTERFACE).ok()?;
    let list: Vec<(String, String, String)> = proxy.call("getLayoutsList", &()).ok()?;

    let codes: Vec<String> = list
        .into_iter()
        .map(|(short, _variant, _name)| short.to_ascii_uppercase())
        .filter(|code| !code.is_empty())
        .collect();

    (!codes.is_empty()).then_some(codes)
}

#[cfg(test)]
mod tests {
    /// Проверка живого сервиса — только если он есть в сессии.
    #[test]
    fn live_service_returns_something() {
        if let Some(codes) = super::layout_codes() {
            assert!(!codes.is_empty());
            assert!(codes.iter().all(|c| !c.is_empty()));
        }
    }
}
