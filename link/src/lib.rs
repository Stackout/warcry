pub mod auth;
pub mod codec;
pub mod proto;

pub use auth::{Denied, Gate, Pairing, Token};
pub use codec::{CodecError, MAX_FRAME, decode, encode};
pub use proto::*;

pub const PROTOCOL: u16 = 1;

pub const SERVICE: &str = "_warcry._udp.local.";

pub const ALPN: &[u8] = b"warcry/1";

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
