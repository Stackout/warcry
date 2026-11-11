use super::WindowInfo;
use anyhow::{Context, Result};
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ClientMessageEvent, ConfigureWindowAux, ConnectionExt, EventMask, StackMode,
    Window,
};
use x11rb::rust_connection::RustConnection;

struct X {
    conn: RustConnection,
    root: Window,
}

impl X {
    fn connect() -> Result<X> {
        let (conn, screen) = x11rb::connect(None)
            .context("can't reach an X11 display; window targeting needs X11 or XWayland")?;
        let root = conn.setup().roots[screen].root;
        Ok(X { conn, root })
    }

    fn atom(&self, name: &str) -> Result<Atom> {
        Ok(self.conn.intern_atom(false, name.as_bytes())?.reply()?.atom)
    }

    fn u32s(&self, win: Window, prop: Atom, ty: impl Into<Atom>) -> Result<Vec<u32>> {
        let reply = self
            .conn
            .get_property(false, win, prop, ty, 0, 8192)?
            .reply()?;
        Ok(reply.value32().map(Iterator::collect).unwrap_or_default())
    }

    fn string(&self, win: Window, prop: Atom, ty: impl Into<Atom>) -> Option<String> {
        let reply = self
            .conn
            .get_property(false, win, prop, ty, 0, 4096)
            .ok()?
            .reply()
            .ok()?;
        (!reply.value.is_empty()).then(|| String::from_utf8_lossy(&reply.value).into_owned())
    }

    fn active_id(&self) -> Result<u32> {
        Ok(self
            .u32s(
                self.root,
                self.atom("_NET_ACTIVE_WINDOW")?,
                AtomEnum::WINDOW,
            )?
            .first()
            .copied()
            .unwrap_or(0))
    }

    fn info(&self, win: Window) -> Result<WindowInfo> {
        let title = self
            .string(win, self.atom("_NET_WM_NAME")?, self.atom("UTF8_STRING")?)
            .or_else(|| self.string(win, AtomEnum::WM_NAME.into(), AtomEnum::STRING))
            .unwrap_or_default();
        let class = self
            .string(win, AtomEnum::WM_CLASS.into(), AtomEnum::STRING)
            .and_then(|c| {
                c.split('\0')
                    .filter(|s| !s.is_empty())
                    .nth(1)
                    .map(String::from)
            })
            .unwrap_or_default();
        let pid = self
            .u32s(win, self.atom("_NET_WM_PID")?, AtomEnum::CARDINAL)?
            .first()
            .copied()
            .unwrap_or(0);
        let process = std::fs::read_to_string(format!("/proc/{pid}/comm"))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        Ok(WindowInfo {
            id: win as u64,
            pid,
            title,
            process,
            class,
        })
    }
}

pub fn list() -> Result<Vec<WindowInfo>> {
    let x = X::connect()?;
    let ids = x.u32s(x.root, x.atom("_NET_CLIENT_LIST")?, AtomEnum::WINDOW)?;
    Ok(ids
        .into_iter()
        .filter_map(|w| x.info(w).ok())
        .filter(|w| !w.title.is_empty())
        .collect())
}

pub fn active() -> Result<Option<WindowInfo>> {
    let x = X::connect()?;
    match x.active_id()? {
        0 => Ok(None),
        w => x.info(w).map(Some),
    }
}

pub fn focus(w: &WindowInfo) -> Result<()> {
    let x = X::connect()?;
    let win = w.id as Window;
    let event = ClientMessageEvent::new(
        32,
        win,
        x.atom("_NET_ACTIVE_WINDOW")?,
        [2u32, 0, x.active_id()?, 0, 0],
    );
    let mask = EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY;
    x.conn.send_event(false, x.root, mask, event)?;
    x.conn
        .configure_window(win, &ConfigureWindowAux::new().stack_mode(StackMode::ABOVE))?;
    x.conn.flush()?;
    Ok(())
}
