use crossbeam_channel::{Receiver, Sender, unbounded};
use std::collections::HashMap;
use warcry_link::{Downlink, FromHost, SessionId, ToHost, Uplink};

#[derive(Default)]
pub struct Switch {
    host: Option<Sender<ToHost>>,
    phones: HashMap<SessionId, Sender<Downlink>>,
    next: SessionId,
}

impl Switch {
    pub fn new() -> Switch {
        Switch::default()
    }

    pub fn has_host(&self) -> bool {
        self.host.is_some()
    }

    pub fn phones(&self) -> usize {
        self.phones.len()
    }

    pub fn host_arrived(&mut self) -> Receiver<ToHost> {
        let (tx, rx) = unbounded();
        self.phones
            .keys()
            .copied()
            .collect::<Vec<SessionId>>()
            .into_iter()
            .for_each(|session| drop(tx.send(ToHost::Joined { session })));
        self.host = Some(tx);
        rx
    }

    pub fn host_left(&mut self) {
        self.host = None;
        self.tell_everyone(Downlink::Denied {
            reason: warcry_link::Denied::NotAuthenticated,
        });
    }

    pub fn join(&mut self) -> (SessionId, Receiver<Downlink>) {
        self.next = self.next.wrapping_add(1);
        let session = self.next;
        let (tx, rx) = unbounded();
        self.phones.insert(session, tx);
        self.to_host(ToHost::Joined { session });
        (session, rx)
    }

    pub fn leave(&mut self, session: SessionId) {
        self.phones.remove(&session);
        self.to_host(ToHost::Left { session });
    }

    pub fn from_phone(&mut self, session: SessionId, message: Uplink) -> bool {
        self.to_host(ToHost::Uplink { session, message })
    }

    pub fn from_host(&mut self, frame: FromHost) {
        let wanted: Vec<SessionId> = self
            .phones
            .keys()
            .copied()
            .filter(|session| frame.wanted_by(*session))
            .collect();
        let message = frame.message();
        wanted.into_iter().for_each(|session| {
            let alive = self
                .phones
                .get(&session)
                .is_some_and(|phone| phone.send(message.clone()).is_ok());
            (!alive).then(|| self.phones.remove(&session));
        });
    }

    fn tell_everyone(&mut self, message: Downlink) {
        self.from_host(FromHost::everyone(message));
    }

    fn to_host(&mut self, frame: ToHost) -> bool {
        let delivered = self
            .host
            .as_ref()
            .is_some_and(|host| host.send(frame).is_ok());
        (!delivered).then(|| self.host = None);
        delivered
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alert() -> Downlink {
        Downlink::Alert {
            rig: 0,
            alert: warcry_link::Alert::Bite { confidence: 0.9 },
            server_ms: 1,
        }
    }

    #[test]
    fn a_phone_with_no_host_is_told_nothing_and_its_uplink_goes_nowhere() {
        let mut switch = Switch::new();
        let (session, phone) = switch.join();
        assert!(!switch.has_host());
        assert!(!switch.from_phone(session, warcry_link::Uplink::Ping { nonce: 1 }));
        assert!(phone.try_recv().is_err());
    }

    #[test]
    fn a_broadcast_reaches_every_phone() {
        let mut switch = Switch::new();
        switch.host_arrived();
        let (_, one) = switch.join();
        let (_, two) = switch.join();
        switch.from_host(FromHost::everyone(alert()));
        assert_eq!(one.try_recv(), Ok(alert()));
        assert_eq!(two.try_recv(), Ok(alert()));
    }

    #[test]
    fn a_targeted_reply_reaches_only_its_phone() {
        let mut switch = Switch::new();
        switch.host_arrived();
        let (first, one) = switch.join();
        let (_, two) = switch.join();
        switch.from_host(FromHost::just(first, alert()));
        assert_eq!(one.try_recv(), Ok(alert()));
        assert!(two.try_recv().is_err());
    }

    #[test]
    fn the_host_hears_about_arrivals_and_departures() {
        let mut switch = Switch::new();
        let host = switch.host_arrived();
        let (session, _phone) = switch.join();
        assert_eq!(host.try_recv(), Ok(ToHost::Joined { session }));
        switch.leave(session);
        assert_eq!(host.try_recv(), Ok(ToHost::Left { session }));
        assert_eq!(switch.phones(), 0);
    }

    #[test]
    fn a_late_host_is_told_who_is_already_waiting() {
        let mut switch = Switch::new();
        let (first, _one) = switch.join();
        let (second, _two) = switch.join();
        let host = switch.host_arrived();
        let mut seen = vec![host.try_recv().unwrap(), host.try_recv().unwrap()];
        seen.sort_by_key(|frame| match frame {
            ToHost::Joined { session } => *session,
            _ => 0,
        });
        assert_eq!(
            seen,
            vec![
                ToHost::Joined { session: first },
                ToHost::Joined { session: second }
            ]
        );
    }

    #[test]
    fn a_phone_press_is_tagged_with_its_session() {
        let mut switch = Switch::new();
        let host = switch.host_arrived();
        let (session, _phone) = switch.join();
        assert_eq!(host.try_recv(), Ok(ToHost::Joined { session }));
        let press = warcry_link::Uplink::Press(warcry_link::Press {
            rig: 2,
            slot: 1,
            seq: 5,
            at_ms: 0,
        });
        assert!(switch.from_phone(session, press.clone()));
        assert_eq!(
            host.try_recv(),
            Ok(ToHost::Uplink {
                session,
                message: press
            })
        );
    }

    #[test]
    fn a_reconnecting_host_replaces_the_old_one() {
        let mut switch = Switch::new();
        let stale = switch.host_arrived();
        let fresh = switch.host_arrived();
        let (session, _phone) = switch.join();
        assert!(stale.try_recv().is_err(), "the stale host still got frames");
        assert_eq!(fresh.try_recv(), Ok(ToHost::Joined { session }));
    }

    #[test]
    fn a_departed_host_is_forgotten_once_its_channel_closes() {
        let mut switch = Switch::new();
        let host = switch.host_arrived();
        drop(host);
        let (session, _phone) = switch.join();
        assert!(!switch.has_host());
        assert!(!switch.from_phone(session, warcry_link::Uplink::Ping { nonce: 1 }));
    }

    #[test]
    fn phones_that_hung_up_are_dropped_on_the_next_broadcast() {
        let mut switch = Switch::new();
        switch.host_arrived();
        let (_, one) = switch.join();
        drop(switch.join().1);
        assert_eq!(switch.phones(), 2);
        switch.from_host(FromHost::everyone(alert()));
        assert_eq!(switch.phones(), 1);
        assert_eq!(one.try_recv(), Ok(alert()));
    }
}
