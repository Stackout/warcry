use crate::auth::{Denied, Token};
use serde::{Deserialize, Serialize};

pub type RigId = u16;

pub type Slot = u8;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Snr {
    Enhanced,
    Mixed,
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigKind {
    #[default]
    Window,
    Deck,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionSlot {
    pub slot: Slot,
    pub label: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct RigInfo {
    pub id: RigId,
    pub kind: RigKind,
    pub label: String,
    pub pid: u32,
    pub actions: Vec<ActionSlot>,
    pub snr: Snr,
    pub ready: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RigState {
    Idle,
    Casting { ends_in_ms: u32 },
    Bite { confidence: f32, age_ms: u32 },
    Caught,
    Expired,
    BagsFull,
    Offline,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Attention {
    Unspecified,
    Whisper,
    HealthLow,
    Disconnected,
    CastFailed,
    PlayerNearby,
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Alert {
    Bite { confidence: f32 },
    BagsFull,
    LureExpiring { secs_left: u16 },
    LureGone,
    CastExpired,
    Caught,
    Attention { reason: Attention },
}

impl Alert {
    pub fn urgency(&self) -> u8 {
        match self {
            Alert::Bite { .. } | Alert::Attention { .. } => 3,
            Alert::BagsFull | Alert::LureGone => 2,
            Alert::LureExpiring { .. } => 1,
            Alert::CastExpired => 1,
            Alert::Caught => 0,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Press {
    pub rig: RigId,
    pub slot: Slot,
    pub seq: u64,
    pub at_ms: u64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Uplink {
    Auth {
        token: Token,
        protocol: u16,
        client: String,
    },
    Listen {
        rig: Option<RigId>,
    },
    Press(Press),
    Calibrate {
        rig: RigId,
        start: bool,
    },
    Ping {
        nonce: u64,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Downlink {
    Welcome {
        server: String,
        protocol: u16,
        server_ms: u64,
        rigs: Vec<RigInfo>,
    },
    Denied {
        reason: Denied,
    },
    Rigs(Vec<RigInfo>),
    State {
        rig: RigId,
        state: RigState,
        server_ms: u64,
    },
    Alert {
        rig: RigId,
        alert: Alert,
        server_ms: u64,
    },
    Pressed {
        rig: RigId,
        slot: Slot,
        seq: u64,
        ok: bool,
    },
    Audio {
        rig: RigId,
        seq: u32,
        frame: Vec<u8>,
    },
    Calibrating {
        rig: RigId,
        have: u8,
        want: u8,
    },
    Pong {
        nonce: u64,
        server_ms: u64,
    },
}

pub type SessionId = u32;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToHost {
    Hello { hub: String, protocol: u16 },
    Joined { session: SessionId },
    Left { session: SessionId },
    Uplink { session: SessionId, message: Uplink },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FromHost {
    Downlink {
        session: Option<SessionId>,
        message: Downlink,
    },
}

impl FromHost {
    pub fn everyone(message: Downlink) -> FromHost {
        FromHost::Downlink {
            session: None,
            message,
        }
    }

    pub fn just(session: SessionId, message: Downlink) -> FromHost {
        FromHost::Downlink {
            session: Some(session),
            message,
        }
    }

    pub fn wanted_by(&self, session: SessionId) -> bool {
        match self {
            FromHost::Downlink { session: None, .. } => true,
            FromHost::Downlink {
                session: Some(only),
                ..
            } => *only == session,
        }
    }

    pub fn message(self) -> Downlink {
        match self {
            FromHost::Downlink { message, .. } => message,
        }
    }
}
