use serde::Serialize;
use serde::de::DeserializeOwned;
use std::fmt;

pub const MAX_FRAME: usize = 16 * 1024;

pub const MAX_DATAGRAM: usize = 1_100;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CodecError {
    TooLarge { len: usize },
    Malformed,
}

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CodecError::TooLarge { len } => write!(f, "message of {len} bytes exceeds {MAX_FRAME}"),
            CodecError::Malformed => write!(f, "malformed message"),
        }
    }
}

impl std::error::Error for CodecError {}

pub fn encode<T: Serialize>(message: &T) -> Result<Vec<u8>, CodecError> {
    let bytes = postcard::to_allocvec(message).map_err(|_| CodecError::Malformed)?;
    match bytes.len() {
        len if len > MAX_FRAME => Err(CodecError::TooLarge { len }),
        _ => Ok(bytes),
    }
}

pub fn decode<T: DeserializeOwned>(bytes: &[u8]) -> Result<T, CodecError> {
    match bytes.len() {
        len if len > MAX_FRAME => Err(CodecError::TooLarge { len }),
        _ => postcard::from_bytes(bytes).map_err(|_| CodecError::Malformed),
    }
}

pub fn fits_datagram(bytes: &[u8]) -> bool {
    bytes.len() <= MAX_DATAGRAM
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Token;
    use crate::proto::*;

    #[test]
    fn press_roundtrips() {
        let press = Press {
            rig: 3,
            slot: 1,
            seq: 9_001,
            at_ms: 1_762_000_000_000,
        };
        let up = Uplink::Press(press);
        let bytes = encode(&up).unwrap();
        assert_eq!(decode::<Uplink>(&bytes), Ok(up));
    }

    #[test]
    fn a_bite_alert_fits_in_one_datagram() {
        let bytes = encode(&Downlink::Alert {
            rig: 7,
            alert: Alert::Bite { confidence: 0.98 },
            server_ms: 1_762_000_000_000,
        })
        .unwrap();
        assert!(
            fits_datagram(&bytes),
            "bite alert was {} bytes",
            bytes.len()
        );
        assert!(bytes.len() < 32, "bite alert was {} bytes", bytes.len());
    }

    #[test]
    fn a_press_fits_in_one_datagram() {
        let bytes = encode(&Uplink::Press(Press {
            rig: 2,
            slot: 0,
            seq: u64::MAX,
            at_ms: u64::MAX,
        }))
        .unwrap();
        assert!(fits_datagram(&bytes));
        assert!(bytes.len() < 32, "press was {} bytes", bytes.len());
    }

    #[test]
    fn auth_roundtrips_with_its_token() {
        let up = Uplink::Auth {
            token: Token::from_bytes([4; 32]),
            protocol: crate::PROTOCOL,
            client: "ryan's iphone".into(),
        };
        let bytes = encode(&up).unwrap();
        assert_eq!(decode::<Uplink>(&bytes), Ok(up));
    }

    #[test]
    fn a_tagged_uplink_roundtrips() {
        let framed = ToHost::Uplink {
            session: 42,
            message: Uplink::Press(Press {
                rig: 1,
                slot: 0,
                seq: 7,
                at_ms: 9,
            }),
        };
        let bytes = encode(&framed).unwrap();
        assert_eq!(decode::<ToHost>(&bytes), Ok(framed));
        assert!(fits_datagram(&bytes));
    }

    #[test]
    fn a_broadcast_goes_to_every_session_and_a_reply_to_one() {
        let shout = FromHost::everyone(Downlink::Alert {
            rig: 0,
            alert: Alert::Bite { confidence: 0.9 },
            server_ms: 1,
        });
        assert!(shout.wanted_by(1));
        assert!(shout.wanted_by(99));
        let private = FromHost::just(
            7,
            Downlink::Pressed {
                rig: 0,
                slot: 0,
                seq: 1,
                ok: true,
            },
        );
        assert!(private.wanted_by(7));
        assert!(!private.wanted_by(8));
    }

    #[test]
    fn host_frames_survive_json_too() {
        let framed = ToHost::Joined { session: 3 };
        let json = serde_json::to_string(&framed).unwrap();
        assert_eq!(serde_json::from_str::<ToHost>(&json).unwrap(), framed);
    }

    #[test]
    fn garbage_is_rejected_not_panicked_on() {
        assert_eq!(decode::<Uplink>(&[0xff; 64]), Err(CodecError::Malformed));
        assert_eq!(decode::<Uplink>(&[]), Err(CodecError::Malformed));
    }

    #[test]
    fn oversized_input_is_refused_before_decoding() {
        let huge = vec![0u8; MAX_FRAME + 1];
        assert_eq!(
            decode::<Uplink>(&huge),
            Err(CodecError::TooLarge { len: huge.len() })
        );
    }
}
