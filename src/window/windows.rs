use super::WindowInfo;
use crate::input::InputSink;
use crate::keys::Key;
use crate::profile::MouseButton;
use anyhow::{Result, anyhow, ensure};
use windows_sys::Win32::Foundation::{CloseHandle, HWND, LPARAM, RECT, WPARAM};
use windows_sys::Win32::System::Threading::{
    AttachThreadInput, GetCurrentThreadId, OpenProcess, PROCESS_NAME_WIN32,
    PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    KEYEVENTF_KEYUP, MAPVK_VK_TO_VSC, MapVirtualKeyW, VkKeyScanW, keybd_event,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    BringWindowToTop, EnumWindows, GetClassNameW, GetClientRect, GetForegroundWindow,
    GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId, IsIconic, IsWindow,
    IsWindowVisible, PostMessageW, SW_RESTORE, SetForegroundWindow, ShowWindow, WM_CHAR,
    WM_KEYDOWN, WM_KEYUP, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MBUTTONDOWN, WM_MBUTTONUP,
    WM_MOUSEHWHEEL, WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_RBUTTONDOWN, WM_RBUTTONUP, WM_SYSKEYDOWN,
    WM_SYSKEYUP, WM_XBUTTONDOWN, WM_XBUTTONUP,
};
use windows_sys::core::BOOL;

fn hwnd(id: u64) -> HWND {
    id as usize as HWND
}

fn process_name(pid: u32) -> String {
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    let read = || {
        let mut buf = [0u16; 1024];
        let mut len = buf.len() as u32;
        let ok = unsafe {
            QueryFullProcessImageNameW(handle, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len)
        };
        let path = String::from_utf16_lossy(&buf[..len as usize]);
        unsafe { CloseHandle(handle) };
        (ok != 0).then(|| {
            path.rsplit(['\\', '/'])
                .next()
                .unwrap_or_default()
                .to_string()
        })
    };
    (!handle.is_null()).then(read).flatten().unwrap_or_default()
}

fn info(h: HWND) -> Option<WindowInfo> {
    let len = unsafe { GetWindowTextLengthW(h) };
    (len > 0).then(|| {
        let mut title = vec![0u16; len as usize + 1];
        let mut class = [0u16; 256];
        let mut pid = 0u32;
        let n = unsafe { GetWindowTextW(h, title.as_mut_ptr(), title.len() as i32) };
        let cn = unsafe { GetClassNameW(h, class.as_mut_ptr(), class.len() as i32) };
        unsafe { GetWindowThreadProcessId(h, &mut pid) };
        WindowInfo {
            id: h as usize as u64,
            pid,
            title: String::from_utf16_lossy(&title[..n.max(0) as usize]),
            process: process_name(pid),
            class: String::from_utf16_lossy(&class[..cn.max(0) as usize]),
        }
    })
}

unsafe extern "system" fn collect(h: HWND, out: LPARAM) -> BOOL {
    let out = unsafe { &mut *(out as *mut Vec<WindowInfo>) };
    if let Some(w) = (unsafe { IsWindowVisible(h) } != 0)
        .then(|| info(h))
        .flatten()
    {
        out.push(w);
    }
    1
}

pub fn list() -> Result<Vec<WindowInfo>> {
    let mut out: Vec<WindowInfo> = Vec::new();
    unsafe { EnumWindows(Some(collect), &mut out as *mut Vec<WindowInfo> as LPARAM) };
    Ok(out)
}

pub fn active() -> Result<Option<WindowInfo>> {
    let h = unsafe { GetForegroundWindow() };
    Ok((!h.is_null()).then(|| info(h)).flatten())
}

pub fn focus(w: &WindowInfo) -> Result<()> {
    let target = hwnd(w.id);
    ensure!(
        unsafe { IsWindow(target) } != 0,
        "window {w} no longer exists"
    );
    unsafe {
        (IsIconic(target) != 0).then(|| ShowWindow(target, SW_RESTORE));
        let fg_thread = GetWindowThreadProcessId(GetForegroundWindow(), std::ptr::null_mut());
        let me = GetCurrentThreadId();
        let attached =
            fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, 1) != 0;
        BringWindowToTop(target);
        (SetForegroundWindow(target) == 0).then(|| {
            keybd_event(0x12, 0, 0, 0);
            keybd_event(0x12, 0, KEYEVENTF_KEYUP, 0);
            SetForegroundWindow(target)
        });
        attached.then(|| AttachThreadInput(me, fg_thread, 0));
    }
    Ok(())
}

pub struct BackgroundSink {
    hwnd: usize,
    cursor: Option<(i32, i32)>,
    held: u32,
}

impl BackgroundSink {
    pub fn new(w: &WindowInfo) -> Self {
        Self {
            hwnd: w.id as usize,
            cursor: None,
            held: 0,
        }
    }

    fn post(&self, msg: u32, wparam: WPARAM, lparam: LPARAM) -> Result<()> {
        let ok = unsafe { PostMessageW(self.hwnd as HWND, msg, wparam, lparam) };
        ensure!(ok != 0, "PostMessage failed (window closed?)");
        Ok(())
    }

    fn point(&self) -> LPARAM {
        let center = || {
            let mut r = RECT {
                left: 0,
                top: 0,
                right: 0,
                bottom: 0,
            };
            unsafe { GetClientRect(self.hwnd as HWND, &mut r) };
            ((r.right - r.left) / 2, (r.bottom - r.top) / 2)
        };
        let (x, y) = self.cursor.unwrap_or_else(center);
        (((y as u32 & 0xFFFF) << 16) | (x as u32 & 0xFFFF)) as LPARAM
    }
}

fn virtual_key(key: Key) -> Result<u32> {
    let layout = |c: char| {
        let unit = c.encode_utf16(&mut [0u16; 2])[0];
        let vk = unsafe { VkKeyScanW(unit) };
        (vk != -1).then_some((vk as u16 & 0xFF) as u32)
    };
    key.windows_vk()
        .map(u32::from)
        .or_else(|| match key {
            Key::Char(c) => layout(c),
            _ => None,
        })
        .ok_or_else(|| anyhow!("key {key} isn't on the current keyboard layout"))
}

fn extended(key: Key) -> bool {
    use Key::*;
    matches!(
        key,
        Insert
            | Delete
            | Home
            | End
            | PageUp
            | PageDown
            | Up
            | Down
            | Left
            | Right
            | RCtrl
            | RAlt
            | Meta
    )
}

impl InputSink for BackgroundSink {
    fn key(&mut self, key: Key, down: bool) -> Result<()> {
        let vk = virtual_key(key)?;
        let scan = unsafe { MapVirtualKeyW(vk, MAPVK_VK_TO_VSC) };
        let lparam =
            1 | (scan << 16) | ((extended(key) as u32) << 24) | ((!down as u32) * (0b11 << 30));
        let alt = matches!(key, Key::Alt | Key::LAlt | Key::RAlt);
        let msg = match (down, alt) {
            (true, false) => WM_KEYDOWN,
            (true, true) => WM_SYSKEYDOWN,
            (false, false) => WM_KEYUP,
            (false, true) => WM_SYSKEYUP,
        };
        self.post(msg, vk as WPARAM, lparam as i32 as LPARAM)
    }

    fn button(&mut self, button: MouseButton, down: bool) -> Result<()> {
        let (mask, press, release, xbutton) = match button {
            MouseButton::Left => (0x01, WM_LBUTTONDOWN, WM_LBUTTONUP, 0u32),
            MouseButton::Right => (0x02, WM_RBUTTONDOWN, WM_RBUTTONUP, 0),
            MouseButton::Middle => (0x10, WM_MBUTTONDOWN, WM_MBUTTONUP, 0),
            MouseButton::Back => (0x20, WM_XBUTTONDOWN, WM_XBUTTONUP, 1),
            MouseButton::Forward => (0x40, WM_XBUTTONDOWN, WM_XBUTTONUP, 2),
        };
        self.held = match down {
            true => self.held | mask,
            false => self.held & !mask,
        };
        let msg = [release, press][down as usize];
        self.post(msg, (self.held | (xbutton << 16)) as WPARAM, self.point())
    }

    fn text(&mut self, text: &str) -> Result<()> {
        text.encode_utf16()
            .try_for_each(|unit| self.post(WM_CHAR, unit as WPARAM, 1))
    }

    fn move_mouse(&mut self, x: i32, y: i32, relative: bool) -> Result<()> {
        let (cx, cy) = self.cursor.unwrap_or_default();
        self.cursor = Some([(x, y), (cx + x, cy + y)][relative as usize]);
        self.post(WM_MOUSEMOVE, self.held as WPARAM, self.point())
    }

    fn scroll(&mut self, amount: i32, horizontal: bool) -> Result<()> {
        let delta = [-amount, amount][horizontal as usize] * 120;
        let msg = [WM_MOUSEWHEEL, WM_MOUSEHWHEEL][horizontal as usize];
        self.post(
            msg,
            ((((delta as u32) & 0xFFFF) << 16) | self.held) as WPARAM,
            self.point(),
        )
    }
}
