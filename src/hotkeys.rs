use crate::actions::Shared;
use crate::fault;
use crate::keys::Key;
use crate::profile::Profile;
use anyhow::{Result, anyhow};
use crossbeam_channel::Sender;
use device_query::{DeviceQuery, DeviceState, Keycode};
use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Event {
    Command(usize),
    PushToTalk(bool),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum Id {
    Ptt,
    Command(usize),
}

type Combo = Vec<Vec<Keycode>>;

fn combo(spec: &str) -> Result<Combo> {
    Key::parse_combo(spec)?
        .into_iter()
        .map(Key::to_device_query)
        .collect()
}

fn bindings(profile: &Profile) -> Result<Vec<(Id, Combo)>> {
    let ptt = profile
        .settings
        .push_to_talk
        .as_deref()
        .map(|s| Ok((Id::Ptt, combo(s)?)));
    let commands = profile
        .commands
        .iter()
        .enumerate()
        .filter_map(|(i, c)| c.hotkey.as_deref().map(|h| Ok((Id::Command(i), combo(h)?))));
    ptt.into_iter().chain(commands).collect()
}

fn held(bindings: &[(Id, Combo)], keys: &[Keycode]) -> HashSet<Id> {
    bindings
        .iter()
        .filter(|(_, combo)| {
            combo
                .iter()
                .all(|alts| alts.iter().any(|k| keys.contains(k)))
        })
        .map(|(id, _)| *id)
        .collect()
}

pub fn spawn(shared: Arc<Shared>, tx: Sender<Event>) {
    let needed = |p: &Profile| {
        p.settings.push_to_talk.is_some() || p.commands.iter().any(|c| c.hotkey.is_some())
    };
    needed(&shared.profile()).then(|| fault::spawn("hotkeys", move || listen(&shared, &tx)));
}

fn listen(shared: &Shared, tx: &Sender<Event>) -> Result<()> {
    let device = DeviceState::checked_new().ok_or_else(|| {
        anyhow!(
            "can't read the keyboard (needs X11 on Linux, Input Monitoring permission on macOS)"
        )
    })?;
    let mut profile = shared.profile();
    let mut binds = bindings(&profile)?;
    let mut previous = HashSet::new();
    while shared.alive() {
        let latest = shared.profile();
        if !Arc::ptr_eq(&latest, &profile) {
            binds = bindings(&latest)?;
        }
        profile = latest;
        let current = held(&binds, &device.get_keys());
        let pressed = current.difference(&previous).map(|id| match id {
            Id::Ptt => Event::PushToTalk(true),
            Id::Command(i) => Event::Command(*i),
        });
        let released = previous
            .difference(&current)
            .filter_map(|id| (*id == Id::Ptt).then_some(Event::PushToTalk(false)));
        pressed.chain(released).try_for_each(|e| tx.send(e))?;
        previous = current;
        std::thread::sleep(Duration::from_millis(12));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combos_require_all_keys() {
        let binds = vec![
            (Id::Ptt, combo("ctrl+f9").unwrap()),
            (Id::Command(0), combo("f9").unwrap()),
        ];
        assert_eq!(
            held(&binds, &[Keycode::F9]),
            HashSet::from([Id::Command(0)])
        );
        assert_eq!(
            held(&binds, &[Keycode::RControl, Keycode::F9]),
            HashSet::from([Id::Ptt, Id::Command(0)])
        );
        assert!(held(&binds, &[Keycode::LControl]).is_empty());
    }
}
