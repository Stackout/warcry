use crate::Rigs;
use crossbeam_channel::Sender;
use std::collections::HashMap;
use warcry_link::{Denied, Downlink, Gate, Press, SessionId, Token, Uplink, now_ms};

pub struct Hosted {
    token: Token,
    trust: bool,
    rigs: Rigs,
    presses: Sender<Press>,
    gates: HashMap<SessionId, Gate>,
}

impl Hosted {
    pub fn new(token: Token, rigs: Rigs, presses: Sender<Press>) -> Hosted {
        Hosted {
            token,
            trust: false,
            rigs,
            presses,
            gates: HashMap::new(),
        }
    }

    pub fn trusting(rigs: Rigs, presses: Sender<Press>) -> Hosted {
        Hosted {
            token: Token::from_bytes([0; 32]),
            trust: true,
            rigs,
            presses,
            gates: HashMap::new(),
        }
    }

    pub fn joined(&mut self, session: SessionId) {
        let gate = match self.trust {
            true => Gate::trusting(now_ms()),
            false => Gate::new(self.token, now_ms()),
        };
        self.gates.insert(session, gate);
    }

    pub fn left(&mut self, session: SessionId) {
        self.gates.remove(&session);
    }

    pub fn sessions(&self) -> usize {
        self.gates.len()
    }

    pub fn authenticated(&self, session: SessionId) -> bool {
        self.gates.get(&session).is_some_and(Gate::is_authenticated)
    }

    pub fn expired(&self, session: SessionId) -> bool {
        self.gates
            .get(&session)
            .is_some_and(|gate| gate.handshake_expired(now_ms()))
    }

    pub fn uplink(&mut self, session: SessionId, message: Uplink) -> Option<Downlink> {
        self.gates.contains_key(&session).then_some(())?;
        match message {
            Uplink::Auth {
                token, protocol, ..
            } => Some(self.welcome(session, &token, protocol)),
            Uplink::Press(press) => Some(self.admit(session, press)),
            Uplink::Ping { nonce } => Some(Downlink::Pong {
                nonce,
                server_ms: now_ms(),
            }),
            Uplink::Listen { .. } | Uplink::Calibrate { .. } => None,
        }
    }

    fn welcome(&mut self, session: SessionId, token: &Token, protocol: u16) -> Downlink {
        let checked = self
            .gates
            .get_mut(&session)
            .map(|gate| gate.authenticate(token, protocol, now_ms()));
        match checked {
            None => Downlink::Denied {
                reason: Denied::NotAuthenticated,
            },
            Some(Err(reason)) => Downlink::Denied { reason },
            Some(Ok(())) => Downlink::Welcome {
                server: "warcry".into(),
                protocol: warcry_link::PROTOCOL,
                server_ms: now_ms(),
                rigs: (self.rigs)(),
            },
        }
    }

    fn admit(&mut self, session: SessionId, press: Press) -> Downlink {
        let gated = self
            .gates
            .get_mut(&session)
            .map(|gate| gate.admit(&press, now_ms()));
        let refused = match gated {
            None => Some(Denied::NotAuthenticated),
            Some(Err(reason)) => Some(reason),
            Some(Ok(())) => self.offered(&press).err(),
        };
        refused
            .map(|reason| Downlink::Denied { reason })
            .unwrap_or_else(|| {
                let delivered = self.presses.send(press).is_ok();
                tracing::info!("press slot {} on rig {}", press.slot, press.rig);
                Downlink::Pressed {
                    rig: press.rig,
                    slot: press.slot,
                    seq: press.seq,
                    ok: delivered,
                }
            })
    }

    fn offered(&self, press: &Press) -> Result<(), Denied> {
        let rig = (self.rigs)()
            .into_iter()
            .find(|rig| rig.id == press.rig)
            .ok_or(Denied::UnknownRig)?;
        rig.actions
            .iter()
            .any(|action| action.slot == press.slot)
            .then_some(())
            .ok_or(Denied::UnknownSlot)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crossbeam_channel::unbounded;
    use std::sync::Arc;
    use warcry_link::{ActionSlot, RigInfo, Snr};

    fn rig() -> RigInfo {
        RigInfo {
            id: 1,
            label: "Thrallfisher".into(),
            pid: 10,
            actions: vec![ActionSlot {
                slot: 0,
                label: "reel".into(),
            }],
            snr: Snr::Enhanced,
            ready: true,
        }
    }

    fn hosted() -> (Hosted, Token, crossbeam_channel::Receiver<Press>) {
        let token = Token::from_bytes([3; 32]);
        let (tx, rx) = unbounded();
        let hosted = Hosted::new(token, Arc::new(|| vec![rig()]), tx);
        (hosted, token, rx)
    }

    fn press(seq: u64) -> Uplink {
        Uplink::Press(Press {
            rig: 1,
            slot: 0,
            seq,
            at_ms: 0,
        })
    }

    fn auth(token: Token) -> Uplink {
        Uplink::Auth {
            token,
            protocol: warcry_link::PROTOCOL,
            client: "phone".into(),
        }
    }

    #[test]
    fn a_trusting_host_lets_a_press_through_without_a_token() {
        let (tx, presses) = unbounded();
        let mut hosted = Hosted::trusting(Arc::new(|| vec![rig()]), tx);
        hosted.joined(1);
        assert!(hosted.authenticated(1));
        assert!(matches!(
            hosted.uplink(1, press(1)),
            Some(Downlink::Pressed { ok: true, .. })
        ));
        assert!(presses.try_recv().is_ok());
    }

    #[test]
    fn a_trusting_host_still_refuses_an_unoffered_slot() {
        let (tx, presses) = unbounded();
        let mut hosted = Hosted::trusting(Arc::new(|| vec![rig()]), tx);
        hosted.joined(1);
        let odd = Uplink::Press(Press {
            rig: 1,
            slot: 7,
            seq: 1,
            at_ms: 0,
        });
        assert!(matches!(
            hosted.uplink(1, odd),
            Some(Downlink::Denied {
                reason: Denied::UnknownSlot
            })
        ));
        assert!(presses.try_recv().is_err());
    }

    #[test]
    fn an_unknown_session_is_ignored_entirely() {
        let (mut hosted, token, presses) = hosted();
        assert_eq!(hosted.uplink(9, auth(token)), None);
        assert!(presses.try_recv().is_err());
    }

    #[test]
    fn a_good_token_welcomes_and_lists_rigs() {
        let (mut hosted, token, _) = hosted();
        hosted.joined(1);
        assert!(matches!(
            hosted.uplink(1, auth(token)),
            Some(Downlink::Welcome { rigs, .. }) if rigs == vec![rig()]
        ));
        assert!(hosted.authenticated(1));
    }

    #[test]
    fn a_bad_token_is_refused() {
        let (mut hosted, _, _) = hosted();
        hosted.joined(1);
        assert!(matches!(
            hosted.uplink(1, auth(Token::from_bytes([9; 32]))),
            Some(Downlink::Denied {
                reason: Denied::BadToken
            })
        ));
    }

    #[test]
    fn an_unauthenticated_press_never_reaches_the_desktop() {
        let (mut hosted, _, presses) = hosted();
        hosted.joined(1);
        assert!(matches!(
            hosted.uplink(1, press(1)),
            Some(Downlink::Denied {
                reason: Denied::NotAuthenticated
            })
        ));
        assert!(presses.try_recv().is_err());
    }

    #[test]
    fn an_authenticated_press_reaches_the_desktop_once() {
        let (mut hosted, token, presses) = hosted();
        hosted.joined(1);
        hosted.uplink(1, auth(token));
        assert!(matches!(
            hosted.uplink(1, press(1)),
            Some(Downlink::Pressed { ok: true, .. })
        ));
        assert!(presses.try_recv().is_ok());
        assert!(matches!(
            hosted.uplink(1, press(1)),
            Some(Downlink::Denied {
                reason: Denied::Replay
            })
        ));
        assert!(presses.try_recv().is_err());
    }

    #[test]
    fn one_phone_cannot_use_another_phones_sequence_numbers() {
        let (mut hosted, token, presses) = hosted();
        hosted.joined(1);
        hosted.joined(2);
        hosted.uplink(1, auth(token));
        hosted.uplink(2, auth(token));
        assert!(matches!(
            hosted.uplink(1, press(5)),
            Some(Downlink::Pressed { .. })
        ));
        assert!(matches!(
            hosted.uplink(2, press(5)),
            Some(Downlink::Pressed { .. })
        ));
        assert_eq!(presses.try_iter().count(), 2);
    }

    #[test]
    fn leaving_forgets_the_session_and_its_authentication() {
        let (mut hosted, token, _) = hosted();
        hosted.joined(1);
        hosted.uplink(1, auth(token));
        hosted.left(1);
        assert_eq!(hosted.sessions(), 0);
        assert!(!hosted.authenticated(1));
        assert_eq!(hosted.uplink(1, press(1)), None);
    }

    #[test]
    fn an_unoffered_slot_is_refused() {
        let (mut hosted, token, presses) = hosted();
        hosted.joined(1);
        hosted.uplink(1, auth(token));
        let odd = Uplink::Press(Press {
            rig: 1,
            slot: 7,
            seq: 1,
            at_ms: 0,
        });
        assert!(matches!(
            hosted.uplink(1, odd),
            Some(Downlink::Denied {
                reason: Denied::UnknownSlot
            })
        ));
        assert!(presses.try_recv().is_err());
    }

    #[test]
    fn ping_is_answered_even_before_authentication() {
        let (mut hosted, _, _) = hosted();
        hosted.joined(1);
        assert!(matches!(
            hosted.uplink(1, Uplink::Ping { nonce: 4 }),
            Some(Downlink::Pong { nonce: 4, .. })
        ));
    }
}
