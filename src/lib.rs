pub mod actions;
pub mod audio;
pub mod engine;
pub mod fault;
pub mod hotkeys;
pub mod input;
pub mod keys;
pub mod matcher;
pub mod models;
pub mod phrase;
pub mod profile;
pub mod speech_out;
pub mod stt;
pub mod window;

pub const EXAMPLE_FISHING: &str = include_str!("../profiles/fishing.toml");
pub const EXAMPLE_STARTER: &str = include_str!("../profiles/starter.toml");
