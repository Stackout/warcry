use crate::keys::Key;
use crate::phrase;
use anyhow::{Context, Result, ensure};
use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Profile {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default)]
    pub target: Option<Target>,
    #[serde(default, rename = "command")]
    pub commands: Vec<Command>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields, default)]
pub struct Settings {
    pub min_confidence: f64,
    pub push_to_talk: Option<String>,
    pub push_to_talk_mode: PttMode,
    pub start_listening: bool,
    pub model: String,
    pub language: String,
    pub input_device: Option<String>,
    pub vad_threshold: f32,
    pub silence_ms: u32,
    pub max_utterance_ms: u32,
    pub speak_confirmations: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            min_confidence: 0.78,
            push_to_talk: None,
            push_to_talk_mode: PttMode::Hold,
            start_listening: true,
            model: "base.en".into(),
            language: "en".into(),
            input_device: None,
            vad_threshold: 0.012,
            silence_ms: 450,
            max_utterance_ms: 8000,
            speak_confirmations: false,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PttMode {
    #[default]
    Hold,
    Toggle,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Target {
    pub title: Option<String>,
    pub process: Option<String>,
    pub class: Option<String>,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub mode: TargetMode,
}

impl Target {
    pub fn describe(&self) -> String {
        [
            ("title", &self.title),
            ("process", &self.process),
            ("class", &self.class),
        ]
        .iter()
        .filter_map(|(k, v)| v.as_ref().map(|v| format!("{k}~{v:?}")))
        .chain(self.pid.map(|pid| format!("pid {pid}")))
        .collect::<Vec<_>>()
        .join(" ")
    }

    fn check(&self) -> Result<()> {
        ensure!(
            self.title.is_some()
                || self.process.is_some()
                || self.class.is_some()
                || self.pid.is_some(),
            "target needs a window, title, process, or class"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum TargetMode {
    #[default]
    FocusRestore,
    Focus,
    Background,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Command {
    pub name: Option<String>,
    pub phrase: Option<String>,
    pub hotkey: Option<String>,
    pub target: Option<Target>,
    #[serde(default)]
    pub no_target: bool,
    #[serde(default)]
    pub always_active: bool,
    #[serde(default)]
    pub pad: bool,
    #[serde(default)]
    pub stop_others: bool,
    #[serde(default)]
    pub actions: Vec<Action>,
}

impl Command {
    pub fn display_name(&self) -> String {
        [&self.name, &self.phrase, &self.hotkey]
            .into_iter()
            .find_map(Clone::clone)
            .unwrap_or_else(|| "<unnamed>".into())
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseButton {
    #[default]
    Left,
    Right,
    Middle,
    Back,
    Forward,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Switch {
    On,
    Off,
    Toggle,
}

fn one() -> u32 {
    1
}

#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
#[serde(tag = "do", rename_all = "snake_case", deny_unknown_fields)]
pub enum Action {
    Press {
        key: String,
        #[serde(default)]
        hold_ms: u64,
    },
    KeyDown {
        key: String,
    },
    KeyUp {
        key: String,
    },
    Combo {
        keys: String,
        #[serde(default)]
        hold_ms: u64,
    },
    Type {
        text: String,
    },
    Click {
        #[serde(default)]
        button: MouseButton,
        #[serde(default = "one")]
        count: u32,
        #[serde(default)]
        hold_ms: u64,
    },
    MouseDown {
        #[serde(default)]
        button: MouseButton,
    },
    MouseUp {
        #[serde(default)]
        button: MouseButton,
    },
    MouseMove {
        x: i32,
        y: i32,
        #[serde(default)]
        relative: bool,
    },
    Scroll {
        amount: i32,
        #[serde(default)]
        horizontal: bool,
    },
    Wait {
        ms: u64,
    },
    WaitRandom {
        min_ms: u64,
        max_ms: u64,
    },
    Repeat {
        times: Option<u32>,
        actions: Vec<Action>,
    },
    Focus {
        title: Option<String>,
        process: Option<String>,
        class: Option<String>,
    },
    Run {
        program: String,
        #[serde(default)]
        args: Vec<String>,
    },
    Say {
        text: String,
    },
    Log {
        message: String,
    },
    Execute {
        command: String,
    },
    StopAll,
    Listening {
        state: Switch,
    },
}

impl Profile {
    pub fn parse(src: &str) -> Result<Profile> {
        let profile: Profile = toml::from_str(src)?;
        profile.validate()?;
        Ok(profile)
    }

    pub fn load(path: &Path) -> Result<Profile> {
        std::fs::read_to_string(path)
            .with_context(|| format!("reading profile {}", path.display()))
            .and_then(|src| Profile::parse(&src))
            .with_context(|| format!("in profile {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        let text = toml::to_string_pretty(self).context("writing profile")?;
        Profile::parse(&text).context("the edited profile would not load back")?;
        std::fs::write(path, text).with_context(|| format!("saving {}", path.display()))
    }

    pub fn find_command(&self, name: &str) -> Option<usize> {
        let wanted = name.trim().to_lowercase();
        let by_name = |c: &Command| c.name.as_ref().is_some_and(|n| n.to_lowercase() == wanted);
        let by_phrase = |c: &Command| {
            c.phrase
                .as_deref()
                .and_then(|p| phrase::expand(p).ok())
                .is_some_and(|ps| ps.contains(&wanted))
        };
        self.commands
            .iter()
            .position(by_name)
            .or_else(|| self.commands.iter().position(by_phrase))
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(
            (0.0..=1.0).contains(&self.settings.min_confidence),
            "settings.min_confidence must be 0.0-1.0"
        );
        self.settings
            .push_to_talk
            .as_deref()
            .map(check_hotkey)
            .transpose()
            .context("settings.push_to_talk")?;
        self.target
            .as_ref()
            .map(Target::check)
            .transpose()
            .context("[target]")?;
        self.commands.iter().try_for_each(|c| {
            self.check_command(c)
                .with_context(|| format!("command {:?}", c.display_name()))
        })
    }

    fn check_command(&self, c: &Command) -> Result<()> {
        ensure!(
            c.phrase.is_some() || c.hotkey.is_some() || c.name.is_some(),
            "a command needs a phrase, hotkey, or name"
        );
        c.phrase.as_deref().map(phrase::expand).transpose()?;
        c.hotkey.as_deref().map(check_hotkey).transpose()?;
        c.target.as_ref().map(Target::check).transpose()?;
        self.check_actions(&c.actions)
    }

    fn check_actions(&self, actions: &[Action]) -> Result<()> {
        actions.iter().try_for_each(|a| match a {
            Action::Press { key, .. } | Action::KeyDown { key } | Action::KeyUp { key } => {
                Key::parse(key)?.to_enigo().map(drop)
            }
            Action::Combo { keys, .. } => Key::parse_combo(keys)?
                .into_iter()
                .try_for_each(|k| k.to_enigo().map(drop)),
            Action::WaitRandom { min_ms, max_ms } => {
                ensure!(
                    min_ms <= max_ms,
                    "wait_random: min_ms is greater than max_ms"
                );
                Ok(())
            }
            Action::Repeat { actions, .. } => {
                ensure!(!actions.is_empty(), "repeat needs at least one action");
                self.check_actions(actions)
            }
            Action::Focus {
                title,
                process,
                class,
            } => {
                ensure!(
                    title.is_some() || process.is_some() || class.is_some(),
                    "focus needs a title, process, or class"
                );
                Ok(())
            }
            Action::Execute { command } => {
                ensure!(
                    self.find_command(command).is_some(),
                    "execute: no command named {command:?}"
                );
                Ok(())
            }
            _ => Ok(()),
        })
    }
}

fn check_hotkey(s: &str) -> Result<()> {
    Key::parse_combo(s)?
        .into_iter()
        .try_for_each(|k| k.to_device_query().map(drop))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_saved_profile_loads_back_with_its_commands() {
        let path = std::env::temp_dir().join(format!("warcry-save-{}.toml", std::process::id()));
        let mut profile = Profile {
            name: "Pets".into(),
            ..Profile::default()
        };
        profile.commands.push(Command {
            name: Some("heal pet".into()),
            phrase: Some("heal [my;] pet".into()),
            target: Some(Target {
                title: Some("World of Warcraft".into()),
                ..Target::default()
            }),
            actions: vec![Action::Combo {
                keys: "shift+3".into(),
                hold_ms: 0,
            }],
            ..Command::default()
        });
        profile.save(&path).unwrap();
        let back = Profile::load(&path).unwrap();
        assert_eq!(back.commands.len(), 1);
        assert_eq!(back.commands[0].name.as_deref(), Some("heal pet"));
        assert_eq!(
            back.commands[0].actions,
            vec![Action::Combo {
                keys: "shift+3".into(),
                hold_ms: 0
            }]
        );
        std::fs::remove_file(path).ok();
    }

    #[test]
    fn a_profile_that_would_not_load_is_never_written() {
        let path = std::env::temp_dir().join(format!("warcry-bad-{}.toml", std::process::id()));
        let mut profile = Profile::default();
        profile.commands.push(Command {
            name: Some("broken".into()),
            phrase: Some("do it".into()),
            actions: vec![Action::Press {
                key: "nonsense-key".into(),
                hold_ms: 0,
            }],
            ..Command::default()
        });
        assert!(profile.save(&path).is_err());
        assert!(!path.exists(), "a rejected profile was written anyway");
    }

    #[test]
    fn bundled_profiles_parse() {
        [crate::EXAMPLE_FISHING, crate::EXAMPLE_STARTER]
            .iter()
            .for_each(|src| {
                Profile::parse(src).unwrap();
            });
    }

    #[test]
    fn actions_deserialize() {
        let p = Profile::parse(
            r#"
            [target]
            title = "Minecraft"
            mode = "background"

            [[command]]
            phrase = "go"
            actions = [
              { do = "press", key = "e", hold_ms = 50 },
              { do = "repeat", times = 3, actions = [{ do = "click", button = "right" }] },
              { do = "listening", state = "toggle" },
            ]
            "#,
        )
        .unwrap();
        assert_eq!(p.target.unwrap().mode, TargetMode::Background);
        assert_eq!(p.commands[0].actions.len(), 3);
    }

    #[test]
    fn rejects_bad_input() {
        assert!(
            Profile::parse("[[command]]\nphrase='x'\nactions=[{do='press', key='nope'}]").is_err()
        );
        assert!(Profile::parse("[[command]]\nphrase='x'\nactions=[{do='fly'}]").is_err());
        assert!(
            Profile::parse("[[command]]\nphrase='x'\nactions=[{do='execute', command='y'}]")
                .is_err()
        );
        assert!(
            Profile::parse("[[command]]\nphrase='x'\nactions=[{do='repeat', actions=[]}]").is_err()
        );
        assert!(Profile::parse("[target]\nmode='focus'").is_err());
    }
}
