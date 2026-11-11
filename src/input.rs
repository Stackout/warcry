use crate::keys::Key;
use crate::profile::MouseButton;
use anyhow::{Result, anyhow};
use enigo::{Axis, Coordinate, Direction, Enigo, Keyboard, Mouse, Settings};

pub trait InputSink {
    fn key(&mut self, key: Key, down: bool) -> Result<()>;
    fn button(&mut self, button: MouseButton, down: bool) -> Result<()>;
    fn text(&mut self, text: &str) -> Result<()>;
    fn move_mouse(&mut self, x: i32, y: i32, relative: bool) -> Result<()>;
    fn scroll(&mut self, amount: i32, horizontal: bool) -> Result<()>;
    fn note(&mut self, _event: String) {}
}

pub struct EnigoSink(Enigo);

impl EnigoSink {
    pub fn new() -> Result<Self> {
        Enigo::new(&Settings {
            release_keys_when_dropped: true,
            ..Settings::default()
        })
        .map(Self)
        .map_err(|e| anyhow!("input simulation unavailable: {e}"))
    }
}

fn direction(down: bool) -> Direction {
    match down {
        true => Direction::Press,
        false => Direction::Release,
    }
}

fn enigo_button(b: MouseButton) -> enigo::Button {
    match b {
        MouseButton::Left => enigo::Button::Left,
        MouseButton::Right => enigo::Button::Right,
        MouseButton::Middle => enigo::Button::Middle,
        MouseButton::Back => enigo::Button::Back,
        MouseButton::Forward => enigo::Button::Forward,
    }
}

impl InputSink for EnigoSink {
    fn key(&mut self, key: Key, down: bool) -> Result<()> {
        self.0
            .key(key.to_enigo()?, direction(down))
            .map_err(|e| anyhow!("key {key}: {e}"))
    }
    fn button(&mut self, button: MouseButton, down: bool) -> Result<()> {
        self.0
            .button(enigo_button(button), direction(down))
            .map_err(|e| anyhow!("mouse {button:?}: {e}"))
    }
    fn text(&mut self, text: &str) -> Result<()> {
        self.0.text(text).map_err(|e| anyhow!("type text: {e}"))
    }
    fn move_mouse(&mut self, x: i32, y: i32, relative: bool) -> Result<()> {
        let c = [Coordinate::Abs, Coordinate::Rel][relative as usize];
        self.0
            .move_mouse(x, y, c)
            .map_err(|e| anyhow!("move mouse: {e}"))
    }
    fn scroll(&mut self, amount: i32, horizontal: bool) -> Result<()> {
        let a = [Axis::Vertical, Axis::Horizontal][horizontal as usize];
        self.0.scroll(amount, a).map_err(|e| anyhow!("scroll: {e}"))
    }
}

pub struct Tracked<S: InputSink> {
    inner: S,
    keys: Vec<Key>,
    buttons: Vec<MouseButton>,
}

impl<S: InputSink> Tracked<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            keys: Vec::new(),
            buttons: Vec::new(),
        }
    }

    pub fn into_inner(self) -> S {
        self.inner
    }

    pub fn release_all(&mut self) {
        std::mem::take(&mut self.keys)
            .into_iter()
            .rev()
            .for_each(|k| drop(self.inner.key(k, false)));
        std::mem::take(&mut self.buttons)
            .into_iter()
            .for_each(|b| drop(self.inner.button(b, false)));
    }
}

impl<S: InputSink> InputSink for Tracked<S> {
    fn key(&mut self, key: Key, down: bool) -> Result<()> {
        self.inner.key(key, down)?;
        self.keys.retain(|k| *k != key);
        down.then(|| self.keys.push(key));
        Ok(())
    }
    fn button(&mut self, button: MouseButton, down: bool) -> Result<()> {
        self.inner.button(button, down)?;
        self.buttons.retain(|b| *b != button);
        down.then(|| self.buttons.push(button));
        Ok(())
    }
    fn text(&mut self, text: &str) -> Result<()> {
        self.inner.text(text)
    }
    fn move_mouse(&mut self, x: i32, y: i32, relative: bool) -> Result<()> {
        self.inner.move_mouse(x, y, relative)
    }
    fn scroll(&mut self, amount: i32, horizontal: bool) -> Result<()> {
        self.inner.scroll(amount, horizontal)
    }
    fn note(&mut self, event: String) {
        self.inner.note(event)
    }
}

fn updown(down: bool) -> &'static str {
    ["up", "down"][down as usize]
}

#[derive(Default)]
pub struct RecordingSink {
    pub events: Vec<String>,
}

impl InputSink for RecordingSink {
    fn key(&mut self, key: Key, down: bool) -> Result<()> {
        self.events.push(format!("key {key} {}", updown(down)));
        Ok(())
    }
    fn button(&mut self, button: MouseButton, down: bool) -> Result<()> {
        self.events
            .push(format!("mouse {button:?} {}", updown(down)));
        Ok(())
    }
    fn text(&mut self, text: &str) -> Result<()> {
        self.events.push(format!("type {text:?}"));
        Ok(())
    }
    fn move_mouse(&mut self, x: i32, y: i32, relative: bool) -> Result<()> {
        self.events.push(format!(
            "move {x},{y}{}",
            ["", " (relative)"][relative as usize]
        ));
        Ok(())
    }
    fn scroll(&mut self, amount: i32, horizontal: bool) -> Result<()> {
        self.events.push(format!(
            "scroll {amount}{}",
            ["", " (horizontal)"][horizontal as usize]
        ));
        Ok(())
    }
    fn note(&mut self, event: String) {
        self.events.push(event);
    }
}
