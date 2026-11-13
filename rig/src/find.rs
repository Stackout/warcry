use anyhow::{Context, Result};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub pid: u32,
    pub command: String,
}

pub fn wow_processes() -> Result<Vec<Candidate>> {
    let output = Command::new("ps")
        .args(["-Ao", "pid=,comm="])
        .output()
        .context("could not run ps to find World of Warcraft")?;
    Ok(parse(&String::from_utf8_lossy(&output.stdout)))
}

fn parse(listing: &str) -> Vec<Candidate> {
    listing
        .lines()
        .filter_map(|line| {
            let (pid, command) = line.trim_start().split_once(char::is_whitespace)?;
            let pid = pid.trim().parse().ok()?;
            is_wow(command).then(|| Candidate {
                pid,
                command: command.to_string(),
            })
        })
        .collect()
}

const BUNDLE: &str = "/World of Warcraft.app/Contents/MacOS/";

fn is_wow(command: &str) -> bool {
    let name = command.rsplit('/').next().unwrap_or(command).to_lowercase();
    name.starts_with("world of warcraft") && !name.contains("launcher") && command.contains(BUNDLE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_wow_among_other_processes() {
        let listing = "\
  412 /usr/libexec/secinitd
 4821 /Applications/World of Warcraft/_forever_/World of Warcraft.app/Contents/MacOS/World of Warcraft
 4822 /Applications/World of Warcraft/_retail_/World of Warcraft.app/Contents/MacOS/World of Warcraft
  999 /Applications/Firefox.app/Contents/MacOS/firefox
";
        let found = parse(listing);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].pid, 4821);
        assert_eq!(found[1].pid, 4822);
    }

    #[test]
    fn ignores_the_launcher_and_battle_net() {
        let listing = "\
 100 /Applications/World of Warcraft/World of Warcraft Launcher.app/Contents/MacOS/World of Warcraft Launcher
 101 /Applications/Battle.net.app/Contents/MacOS/Battle.net
";
        assert_eq!(parse(listing), vec![]);
    }

    #[test]
    fn a_loose_binary_with_the_right_name_is_not_a_client() {
        let listing = "\
 200 /tmp/scratch/World of Warcraft
 201 /Users/someone/Downloads/World of Warcraft
";
        assert_eq!(parse(listing), vec![]);
    }

    #[test]
    fn garbage_lines_are_skipped() {
        assert_eq!(parse("nonsense\n\n   \n"), vec![]);
    }
}
