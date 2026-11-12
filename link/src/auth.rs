use crate::proto::Press;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

pub const TOKEN_LEN: usize = 32;

pub const HANDSHAKE_MS: u64 = 2_000;

pub const PRESS_BURST: f32 = 12.0;

pub const PRESS_PER_SEC: f32 = 10.0;

#[derive(Clone, Copy, PartialEq, Eq)]
pub struct Token([u8; TOKEN_LEN]);

impl Serialize for Token {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match serializer.is_human_readable() {
            true => serializer.serialize_str(&self.to_hex()),
            false => self.0.serialize(serializer),
        }
    }
}

impl<'de> Deserialize<'de> for Token {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        match deserializer.is_human_readable() {
            true => Token::from_hex(&String::deserialize(deserializer)?)
                .map_err(serde::de::Error::custom),
            false => Ok(Token(<[u8; TOKEN_LEN]>::deserialize(deserializer)?)),
        }
    }
}

impl Token {
    pub fn from_bytes(bytes: [u8; TOKEN_LEN]) -> Self {
        Token(bytes)
    }

    pub fn as_bytes(&self) -> &[u8; TOKEN_LEN] {
        &self.0
    }

    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }

    pub fn from_hex(s: &str) -> Result<Self, PairError> {
        let bytes = unhex(s).ok_or(PairError::Malformed)?;
        let bytes: [u8; TOKEN_LEN] = bytes.try_into().map_err(|_| PairError::Malformed)?;
        Ok(Token(bytes))
    }

    pub fn verify(&self, other: &Token) -> bool {
        self.0
            .iter()
            .zip(other.0.iter())
            .fold(0u8, |acc, (a, b)| acc | (a ^ b))
            == 0
    }
}

impl fmt::Debug for Token {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Token(****)")
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Pairing {
    pub host: String,
    pub port: u16,
    pub token: Token,
    pub fingerprint: [u8; 32],
}

impl Pairing {
    pub fn to_uri(&self) -> String {
        format!(
            "warcry://pair?h={}&p={}&t={}&f={}",
            self.host,
            self.port,
            self.token.to_hex(),
            self.fingerprint
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>(),
        )
    }

    pub fn from_uri(uri: &str) -> Result<Self, PairError> {
        let query = uri
            .strip_prefix("warcry://pair?")
            .ok_or(PairError::NotAPairingUri)?;
        let mut host = None;
        let mut port = None;
        let mut token = None;
        let mut fingerprint = None;
        for pair in query.split('&') {
            let (key, value) = pair.split_once('=').ok_or(PairError::Malformed)?;
            match key {
                "h" => host = Some(value.to_string()),
                "p" => port = Some(value.parse().map_err(|_| PairError::Malformed)?),
                "t" => token = Some(Token::from_hex(value)?),
                "f" => {
                    let bytes = unhex(value).ok_or(PairError::Malformed)?;
                    fingerprint = Some(bytes.try_into().map_err(|_| PairError::Malformed)?);
                }
                _ => return Err(PairError::Malformed),
            }
        }
        Ok(Pairing {
            host: host.ok_or(PairError::Malformed)?,
            port: port.ok_or(PairError::Malformed)?,
            token: token.ok_or(PairError::Malformed)?,
            fingerprint: fingerprint.ok_or(PairError::Malformed)?,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PairError {
    NotAPairingUri,
    Malformed,
}

impl fmt::Display for PairError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PairError::NotAPairingUri => write!(f, "not a warcry pairing link"),
            PairError::Malformed => write!(f, "pairing link is malformed"),
        }
    }
}

impl std::error::Error for PairError {}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Denied {
    NotAuthenticated,
    AlreadyAuthenticated,
    BadToken,
    ProtocolMismatch { server: u16 },
    HandshakeTimeout,
    Replay,
    TooFast,
    UnknownRig,
    UnknownSlot,
}

impl fmt::Display for Denied {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Denied::NotAuthenticated => write!(f, "not authenticated"),
            Denied::AlreadyAuthenticated => write!(f, "already authenticated"),
            Denied::BadToken => write!(f, "pairing token rejected"),
            Denied::ProtocolMismatch { server } => {
                write!(f, "protocol mismatch; relay speaks {server}")
            }
            Denied::HandshakeTimeout => write!(f, "did not authenticate in time"),
            Denied::Replay => write!(f, "replayed press"),
            Denied::TooFast => write!(f, "too many presses"),
            Denied::UnknownRig => write!(f, "no such rig"),
            Denied::UnknownSlot => write!(f, "action not offered for this rig"),
        }
    }
}

impl std::error::Error for Denied {}

pub struct Gate {
    token: Token,
    opened_ms: u64,
    authed: bool,
    last_seq: Option<u64>,
    allowance: f32,
    refilled_ms: u64,
}

impl Gate {
    pub fn new(token: Token, now_ms: u64) -> Self {
        Gate {
            token,
            opened_ms: now_ms,
            authed: false,
            last_seq: None,
            allowance: PRESS_BURST,
            refilled_ms: now_ms,
        }
    }

    pub fn is_authenticated(&self) -> bool {
        self.authed
    }

    pub fn handshake_expired(&self, now_ms: u64) -> bool {
        !self.authed && now_ms.saturating_sub(self.opened_ms) > HANDSHAKE_MS
    }

    pub fn authenticate(
        &mut self,
        offered: &Token,
        protocol: u16,
        now_ms: u64,
    ) -> Result<(), Denied> {
        if self.authed {
            return Err(Denied::AlreadyAuthenticated);
        }
        if self.handshake_expired(now_ms) {
            return Err(Denied::HandshakeTimeout);
        }
        if protocol != crate::PROTOCOL {
            return Err(Denied::ProtocolMismatch {
                server: crate::PROTOCOL,
            });
        }
        if !self.token.verify(offered) {
            return Err(Denied::BadToken);
        }
        self.authed = true;
        Ok(())
    }

    pub fn admit(&mut self, press: &Press, now_ms: u64) -> Result<(), Denied> {
        if !self.authed {
            return Err(Denied::NotAuthenticated);
        }
        if self.last_seq.is_some_and(|seen| press.seq <= seen) {
            return Err(Denied::Replay);
        }
        let elapsed = now_ms.saturating_sub(self.refilled_ms) as f32 / 1000.0;
        self.allowance = (self.allowance + elapsed * PRESS_PER_SEC).min(PRESS_BURST);
        self.refilled_ms = now_ms;
        if self.allowance < 1.0 {
            return Err(Denied::TooFast);
        }
        self.allowance -= 1.0;
        self.last_seq = Some(press.seq);
        Ok(())
    }
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    (s.len() % 2 == 0)
        .then(|| {
            s.as_bytes()
                .chunks(2)
                .map(|pair| {
                    let text = std::str::from_utf8(pair).ok()?;
                    u8::from_str_radix(text, 16).ok()
                })
                .collect::<Option<Vec<u8>>>()
        })
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn token(fill: u8) -> Token {
        Token::from_bytes([fill; TOKEN_LEN])
    }

    fn press(seq: u64) -> Press {
        Press {
            rig: 0,
            slot: 0,
            seq,
            at_ms: 0,
        }
    }

    #[test]
    fn token_is_hex_in_json_and_bytes_in_postcard() {
        let token = Token::from_bytes([0xab; TOKEN_LEN]);
        let json = serde_json::to_string(&token).unwrap();
        assert_eq!(json, format!("\"{}\"", "ab".repeat(TOKEN_LEN)));
        assert_eq!(serde_json::from_str::<Token>(&json).unwrap(), token);
        let binary = postcard::to_allocvec(&token).unwrap();
        assert_eq!(binary.len(), TOKEN_LEN);
        assert_eq!(postcard::from_bytes::<Token>(&binary).unwrap(), token);
    }

    #[test]
    fn malformed_json_token_is_an_error_not_a_panic() {
        assert!(serde_json::from_str::<Token>("\"nothex\"").is_err());
        assert!(serde_json::from_str::<Token>("[1,2,3]").is_err());
    }

    #[test]
    fn token_hex_roundtrip() {
        let t = Token::from_bytes(std::array::from_fn(|i| i as u8));
        assert_eq!(Token::from_hex(&t.to_hex()), Ok(t));
        assert_eq!(Token::from_hex("zz"), Err(PairError::Malformed));
    }

    #[test]
    fn token_never_prints_its_secret() {
        assert_eq!(format!("{:?}", token(0xab)), "Token(****)");
    }

    #[test]
    fn pairing_uri_roundtrip() {
        let p = Pairing {
            host: "100.64.0.7".into(),
            port: 41234,
            token: token(9),
            fingerprint: [7; 32],
        };
        assert_eq!(Pairing::from_uri(&p.to_uri()), Ok(p));
        assert_eq!(
            Pairing::from_uri("https://example.com"),
            Err(PairError::NotAPairingUri)
        );
    }

    #[test]
    fn wrong_token_is_refused() {
        let mut gate = Gate::new(token(1), 0);
        assert_eq!(
            gate.authenticate(&token(2), crate::PROTOCOL, 0),
            Err(Denied::BadToken)
        );
        assert!(!gate.is_authenticated());
    }

    #[test]
    fn protocol_must_match_exactly() {
        let mut gate = Gate::new(token(1), 0);
        assert_eq!(
            gate.authenticate(&token(1), crate::PROTOCOL + 1, 0),
            Err(Denied::ProtocolMismatch {
                server: crate::PROTOCOL
            })
        );
    }

    #[test]
    fn handshake_has_a_deadline() {
        let mut gate = Gate::new(token(1), 0);
        assert_eq!(
            gate.authenticate(&token(1), crate::PROTOCOL, HANDSHAKE_MS + 1),
            Err(Denied::HandshakeTimeout)
        );
    }

    #[test]
    fn press_requires_authentication() {
        let mut gate = Gate::new(token(1), 0);
        assert_eq!(gate.admit(&press(1), 0), Err(Denied::NotAuthenticated));
    }

    #[test]
    fn replayed_press_is_refused() {
        let mut gate = Gate::new(token(1), 0);
        gate.authenticate(&token(1), crate::PROTOCOL, 0).unwrap();
        assert_eq!(gate.admit(&press(5), 0), Ok(()));
        assert_eq!(gate.admit(&press(5), 1), Err(Denied::Replay));
        assert_eq!(gate.admit(&press(4), 2), Err(Denied::Replay));
        assert_eq!(gate.admit(&press(6), 3), Ok(()));
    }

    #[test]
    fn sustained_spam_is_throttled_then_recovers() {
        let mut gate = Gate::new(token(1), 0);
        gate.authenticate(&token(1), crate::PROTOCOL, 0).unwrap();
        let burst: Vec<_> = (1..=40).map(|seq| gate.admit(&press(seq), 0)).collect();
        let allowed = burst.iter().filter(|r| r.is_ok()).count();
        assert_eq!(allowed, PRESS_BURST as usize);
        assert_eq!(burst.last(), Some(&Err(Denied::TooFast)));
        assert_eq!(gate.admit(&press(41), 1_000), Ok(()));
    }
}
