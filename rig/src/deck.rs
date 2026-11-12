use warcry::profile::Profile;
use warcry_link::{ActionSlot, RigId, RigInfo, RigKind, Slot, Snr};

pub const DECK: RigId = RigId::MAX;

const LIMIT: usize = Slot::MAX as usize + 1;

pub fn pads(profile: &Profile) -> Vec<(Slot, usize)> {
    profile
        .commands
        .iter()
        .enumerate()
        .filter(|(_, command)| command.pad)
        .take(LIMIT)
        .enumerate()
        .map(|(slot, (index, _))| (slot as Slot, index))
        .collect()
}

pub fn command(profile: &Profile, slot: Slot) -> Option<usize> {
    pads(profile)
        .into_iter()
        .find(|(at, _)| *at == slot)
        .map(|(_, index)| index)
}

pub fn rig(profile: &Profile) -> Option<RigInfo> {
    let actions: Vec<ActionSlot> = pads(profile)
        .into_iter()
        .filter_map(|(slot, index)| {
            profile.commands.get(index).map(|command| ActionSlot {
                slot,
                label: command.display_name(),
            })
        })
        .collect();
    (!actions.is_empty()).then(|| RigInfo {
        id: DECK,
        kind: RigKind::Deck,
        label: "Commands".into(),
        pid: 0,
        actions,
        snr: Snr::Unknown,
        ready: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use warcry::profile::Command;

    fn command_named(name: &str, pad: bool) -> Command {
        Command {
            name: Some(name.into()),
            pad,
            ..Command::default()
        }
    }

    fn profile() -> Profile {
        Profile {
            commands: vec![
                command_named("cast", false),
                command_named("heal pet", true),
                command_named("stop", false),
                command_named("mount up", true),
            ],
            ..Profile::default()
        }
    }

    #[test]
    fn only_commands_marked_as_pads_get_a_slot() {
        assert_eq!(pads(&profile()), vec![(0, 1), (1, 3)]);
    }

    #[test]
    fn slots_are_dense_so_a_phone_can_lay_them_out() {
        let labels: Vec<String> = rig(&profile())
            .unwrap()
            .actions
            .into_iter()
            .map(|action| format!("{}:{}", action.slot, action.label))
            .collect();
        assert_eq!(labels, vec!["0:heal pet", "1:mount up"]);
    }

    #[test]
    fn a_slot_resolves_back_to_the_command_it_came_from() {
        let profile = profile();
        assert_eq!(command(&profile, 0), Some(1));
        assert_eq!(command(&profile, 1), Some(3));
        assert_eq!(command(&profile, 2), None);
    }

    #[test]
    fn a_profile_without_pads_has_no_deck_at_all() {
        let bare = Profile {
            commands: vec![command_named("cast", false)],
            ..Profile::default()
        };
        assert!(rig(&bare).is_none());
    }

    #[test]
    fn the_deck_is_never_mistaken_for_a_game_window() {
        let deck = rig(&profile()).unwrap();
        assert_eq!(deck.kind, RigKind::Deck);
        assert_eq!(deck.id, DECK);
        assert_eq!(deck.pid, 0);
    }
}
