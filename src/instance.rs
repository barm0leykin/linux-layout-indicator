//! Защита от второго экземпляра: неблокирующий `flock` на lock-файл.
//!
//! Файл лежит в `$XDG_RUNTIME_DIR` (или `/tmp`), блокировка снимается
//! автоматически при завершении процесса — устаревших lock-файлов не бывает.

use std::fs::{File, OpenOptions};
use std::os::unix::io::AsRawFd;
use std::path::PathBuf;

use anyhow::{Context, Result};

pub struct InstanceGuard {
    _file: File,
}

impl InstanceGuard {
    /// Захватить блокировку. Ошибка — если демон уже запущен.
    pub fn acquire() -> Result<Self> {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("/tmp"));
        let path = dir.join("layout-indicator.lock");

        let file = OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(&path)
            .with_context(|| format!("открытие lock-файла {}", path.display()))?;

        // SAFETY: fd валиден, пока живёт `file`.
        let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
        if rc != 0 {
            anyhow::bail!(
                "layout-indicator уже запущен (блокировка {} занята)",
                path.display()
            );
        }
        Ok(Self { _file: file })
    }
}
