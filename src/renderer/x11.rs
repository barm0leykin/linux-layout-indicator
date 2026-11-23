//! X11-оверлей: ARGB-окно с `override_redirect` и пустой область ввода.
//!
//! Механика проверена в спайке (`spike/overlay.c`): окно видно поверх других,
//! клики проходят сквозь, WM его не видит.

use anyhow::{Context, Result};
use tiny_skia::Pixmap;
use x11rb::connection::Connection;
use x11rb::protocol::shape::{ConnectionExt as _, SK, SO};
use x11rb::protocol::xproto::{
    AtomEnum, ClipOrdering, ColormapAlloc, ConnectionExt as _, CreateGCAux, CreateWindowAux,
    EventMask, ImageFormat, ImageOrder, VisualClass, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;

/// Окно-оверлей фиксированного размера.
pub struct X11Overlay {
    conn: RustConnection,
    root: Window,
    screen_num: usize,
    window: Window,
    gc: u32,
    width: u16,
    height: u16,
    lsb_first: bool,
    mapped: bool,
}

impl X11Overlay {
    /// Создать скрытое окно нужного размера.
    pub fn new(width: u32, height: u32) -> Result<Self> {
        let width = u16::try_from(width).context("ширина не влезает в u16")?;
        let height = u16::try_from(height).context("высота не влезает в u16")?;

        let (conn, screen_num) = x11rb::connect(None).context("подключение к X-серверу")?;
        let root = conn.setup().roots[screen_num].root;
        let lsb_first = conn.setup().image_byte_order == ImageOrder::LSB_FIRST;

        // Ищем 32-битный TrueColor-визуал (ARGB).
        let depth = conn.setup().roots[screen_num]
            .allowed_depths
            .iter()
            .find(|d| d.depth == 32)
            .context("X-сервер не предоставляет 32-битный визуал (нет ARGB)")?;
        let visual = depth
            .visuals
            .iter()
            .find(|v| v.class == VisualClass::TRUE_COLOR)
            .context("нет TrueColor-визуала глубины 32")?;

        let colormap = conn.generate_id().context("generate_id colormap")?;
        conn.create_colormap(ColormapAlloc::NONE, colormap, root, visual.visual_id)
            .context("create_colormap")?;

        let window = conn.generate_id().context("generate_id window")?;
        conn.create_window(
            depth.depth,
            window,
            root,
            0,
            0,
            width,
            height,
            0,
            WindowClass::INPUT_OUTPUT,
            visual.visual_id,
            &CreateWindowAux::new()
                .override_redirect(1)
                .background_pixel(0)
                .border_pixel(0)
                .event_mask(EventMask::EXPOSURE)
                .colormap(colormap),
        )
        .context("create_window")?;

        // Пустая область ввода: все клики уходят окну под нами.
        conn.shape_rectangles(
            SO::SET,
            SK::INPUT,
            ClipOrdering::UNSORTED,
            window,
            0,
            0,
            &[],
        )
        .context("shape_rectangles (input)")?;

        let gc = conn.generate_id().context("generate_id gc")?;
        conn.create_gc(gc, window, &CreateGCAux::default())
            .context("create_gc")?;

        conn.flush().context("flush")?;

        Ok(Self {
            conn,
            root,
            screen_num,
            window,
            gc,
            width,
            height,
            lsb_first,
            mapped: false,
        })
    }

    /// Показать буфер в точке (`x`, `y`) — координаты левого верхнего угла окна.
    pub fn draw_at(&mut self, pixmap: &Pixmap, x: i16, y: i16) -> Result<()> {
        if pixmap.width() != u32::from(self.width) || pixmap.height() != u32::from(self.height) {
            anyhow::bail!("размер буфера не совпадает с размером окна");
        }
        if !self.mapped {
            self.conn.map_window(self.window).context("map_window")?;
            self.mapped = true;
        }
        self.conn
            .configure_window(
                self.window,
                &x11rb::protocol::xproto::ConfigureWindowAux::new()
                    .x(i32::from(x))
                    .y(i32::from(y)),
            )
            .context("configure_window")?;

        // RGBA (предумноженный) → порядок байт сервера.
        let mut data = pixmap.data().to_vec();
        if self.lsb_first {
            for px in data.chunks_exact_mut(4) {
                px.swap(0, 2); // R,G,B,A -> B,G,R,A
            }
        }

        self.conn
            .put_image(
                ImageFormat::Z_PIXMAP,
                self.window,
                self.gc,
                self.width,
                self.height,
                0,
                0,
                0,
                32,
                &data,
            )
            .context("put_image")?;
        self.conn.flush().context("flush")?;
        Ok(())
    }

    /// Скрыть окно.
    pub fn hide(&mut self) -> Result<()> {
        if self.mapped {
            self.conn
                .unmap_window(self.window)
                .context("unmap_window")?;
            self.conn.flush().context("flush")?;
            self.mapped = false;
        }
        Ok(())
    }

    /// Идентификатор окна (для диагностики).
    pub fn window_id(&self) -> Window {
        self.window
    }

    /// Есть ли композитор (владелец `_NET_WM_CM_S0`). Без него нет прозрачности.
    pub fn compositor_present(&self) -> bool {
        let Ok(cookie) = self.conn.intern_atom(false, b"_NET_WM_CM_S0") else {
            return false;
        };
        let Ok(reply) = cookie.reply() else {
            return false;
        };
        let Ok(cookie) = self.conn.get_selection_owner(reply.atom) else {
            return false;
        };
        let Ok(owner) = cookie.reply() else {
            return false;
        };
        owner.owner != 0
    }

    /// Позиция указателя в координатах корневого окна.
    pub fn pointer_position(&self) -> Result<(i16, i16)> {
        let reply = self
            .conn
            .query_pointer(self.root)
            .context("query_pointer")?
            .reply()
            .context("ответ query_pointer")?;
        Ok((reply.root_x, reply.root_y))
    }

    /// Размер корневого окна (всей области экранов).
    pub fn root_size(&self) -> (u16, u16) {
        let screen = &self.conn.setup().roots[self.screen_num];
        (screen.width_in_pixels, screen.height_in_pixels)
    }

    /// Активное окно развёрнуто на весь экран (`_NET_WM_STATE_FULLSCREEN`).
    ///
    /// Любая ошибка трактуется как «не fullscreen»: подавление показа —
    /// не та функция, ради которой стоит падать.
    pub fn active_window_is_fullscreen(&self) -> bool {
        let intern = |name: &[u8]| -> Option<u32> {
            self.conn
                .intern_atom(false, name)
                .ok()?
                .reply()
                .ok()
                .map(|r| r.atom)
        };

        let Some(active_atom) = intern(b"_NET_ACTIVE_WINDOW") else {
            return false;
        };
        let Some(state_atom) = intern(b"_NET_WM_STATE") else {
            return false;
        };
        let Some(fullscreen_atom) = intern(b"_NET_WM_STATE_FULLSCREEN") else {
            return false;
        };

        let Ok(cookie) =
            self.conn
                .get_property(false, self.root, active_atom, AtomEnum::WINDOW, 0, 1)
        else {
            return false;
        };
        let Ok(reply) = cookie.reply() else {
            return false;
        };
        let Some(window) = reply.value32().and_then(|mut it| it.next()) else {
            return false;
        };
        if window == 0 {
            return false;
        }

        let Ok(cookie) = self
            .conn
            .get_property(false, window, state_atom, AtomEnum::ATOM, 0, 32)
        else {
            return false;
        };
        let Ok(reply) = cookie.reply() else {
            return false;
        };
        let atoms: Vec<u32> = reply.value32().map(|it| it.collect()).unwrap_or_default();
        crate::log::debug(&format!(
            "fullscreen: window=0x{window:x} atoms={atoms:?} fs_atom={fullscreen_atom} type={:?}",
            reply.type_
        ));
        atoms.contains(&fullscreen_atom)
    }
}
