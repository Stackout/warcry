use crate::addon;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub const NAME_LIMIT: usize = 16;
pub const BODY_LIMIT: usize = 255;

const CACHE: &str = "macros-cache.txt";
const GENERAL: &str = "General";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Macro {
    pub name: String,
    pub icon: String,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Character {
    pub id: String,
    pub name: String,
    pub realm: String,
    pub account: String,
    pub flavor: String,
    pub macros: usize,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Draft {
    pub name: String,
    pub body: String,
}

fn header(line: &str) -> Option<(String, String)> {
    let rest = line.strip_prefix("VER ")?;
    let mut quoted = rest.split('"').skip(1).step_by(2);
    Some((
        quoted.next()?.to_string(),
        quoted.next().unwrap_or_default().to_string(),
    ))
}

pub fn parse(text: &str) -> Vec<Macro> {
    let mut found = Vec::new();
    let mut lines = text.lines();
    while let Some(line) = lines.next() {
        let Some((name, icon)) = header(line) else {
            continue;
        };
        let body: Vec<&str> = lines
            .by_ref()
            .take_while(|rest| rest.trim_end() != "END")
            .collect();
        found.push(Macro {
            name,
            icon,
            body: body.join("\n"),
        });
    }
    found
}

fn label(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn flavor_name(flavor: &Path) -> String {
    let bare = label(flavor);
    let words = bare.trim_matches('_').replace('_', " ");
    let mut chars = words.chars();
    chars
        .next()
        .map(|first| first.to_uppercase().to_string() + chars.as_str())
        .unwrap_or(words)
}

fn subdirs(path: &Path) -> Vec<PathBuf> {
    let mut found: Vec<PathBuf> = std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| label(path) != "SavedVariables")
        .collect();
    found.sort();
    found
}

fn seat(flavor: &str, account: &Path, realm: &str, name: &str, dir: &Path) -> Option<Character> {
    let file = dir.join(CACHE);
    let text = std::fs::read_to_string(&file).ok()?;
    Some(Character {
        id: file.to_string_lossy().to_string(),
        name: name.to_string(),
        realm: realm.to_string(),
        account: label(account),
        flavor: flavor.to_string(),
        macros: parse(&text).len(),
    })
}

fn in_account(flavor: &str, account: &Path) -> Vec<Character> {
    let general = seat(flavor, account, "", GENERAL, account);
    let people = subdirs(account).into_iter().flat_map(|realm| {
        subdirs(&realm)
            .into_iter()
            .filter_map(|who| seat(flavor, account, &label(&realm), &label(&who), &who))
            .collect::<Vec<_>>()
    });
    general.into_iter().chain(people).collect()
}

fn in_flavor(flavor: &Path) -> Vec<Character> {
    let name = flavor_name(flavor);
    subdirs(&flavor.join("WTF").join("Account"))
        .iter()
        .flat_map(|account| in_account(&name, account))
        .collect()
}

pub fn characters() -> Vec<Character> {
    addon::roots()
        .iter()
        .flat_map(|root| addon::flavors(root))
        .flat_map(|flavor| in_flavor(&flavor))
        .collect()
}

pub fn macros_of(id: &str) -> Result<Vec<Macro>> {
    let known = characters()
        .into_iter()
        .find(|c| c.id == id)
        .with_context(|| format!("no character writes macros to {id}"))?;
    let text =
        std::fs::read_to_string(&known.id).with_context(|| format!("reading {}", known.id))?;
    Ok(parse(&text))
}

fn store(home: &Path) -> PathBuf {
    home.join("macros.json")
}

pub fn drafts(home: &Path) -> Vec<Draft> {
    std::fs::read_to_string(store(home))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

pub fn save(home: &Path, drafts: &[Draft]) -> Result<()> {
    std::fs::create_dir_all(home).with_context(|| format!("creating {}", home.display()))?;
    let text = serde_json::to_string_pretty(drafts)?;
    std::fs::write(store(home), text).with_context(|| format!("writing {}", home.display()))
}

pub fn check(draft: &Draft) -> Result<()> {
    anyhow::ensure!(!draft.name.trim().is_empty(), "give the macro a name");
    anyhow::ensure!(
        draft.name.chars().count() <= NAME_LIMIT,
        "World of Warcraft allows {NAME_LIMIT} characters in a macro name, this is {}",
        draft.name.chars().count()
    );
    anyhow::ensure!(
        draft.body.chars().count() <= BODY_LIMIT,
        "World of Warcraft allows {BODY_LIMIT} characters in a macro, this is {}",
        draft.body.chars().count()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "VER 3 0100000000000007 \"Druid\" \"134400\"\n\
        #showtooltip Rejuvenation\n\
        /cast [@player] Rejuvenation\n\
        END\n\
        VER 3 010000000000000C \"Hunter\" \"136104\"\n\
        /cast [@party1,help,nodead] Thorns\n\
        END\n";

    #[test]
    fn a_cache_file_becomes_macros_with_their_bodies() {
        let found = parse(SAMPLE);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].name, "Druid");
        assert_eq!(found[0].icon, "134400");
        assert_eq!(
            found[0].body,
            "#showtooltip Rejuvenation\n/cast [@player] Rejuvenation"
        );
        assert_eq!(found[1].name, "Hunter");
        assert_eq!(found[1].body, "/cast [@party1,help,nodead] Thorns");
    }

    #[test]
    fn a_macro_left_unterminated_still_parses() {
        let found = parse("VER 3 01 \"Last\" \"1\"\n/cast Wrath\n");
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].body, "/cast Wrath");
    }

    #[test]
    fn rubbish_is_not_a_macro() {
        assert!(parse("").is_empty());
        assert!(parse("hello\nworld\n").is_empty());
        assert!(parse("VER 3 01 unquoted\n").is_empty());
    }

    #[test]
    fn a_flavor_folder_reads_as_a_word() {
        assert_eq!(flavor_name(Path::new("/wow/_retail_")), "Retail");
        assert_eq!(flavor_name(Path::new("/wow/_forever_")), "Forever");
        assert_eq!(flavor_name(Path::new("/wow/_classic_era_")), "Classic era");
    }

    #[test]
    fn the_game_s_own_limits_are_enforced() {
        let draft = |name: &str, body: &str| Draft {
            name: name.into(),
            body: body.into(),
        };
        assert!(check(&draft("heal", "/cast Healing Touch")).is_ok());
        assert!(check(&draft("", "/cast Healing Touch")).is_err());
        assert!(check(&draft(&"x".repeat(NAME_LIMIT), "")).is_ok());
        assert!(check(&draft(&"x".repeat(NAME_LIMIT + 1), "")).is_err());
        assert!(check(&draft("heal", &"x".repeat(BODY_LIMIT + 1))).is_err());
    }

    #[test]
    fn drafts_survive_a_round_trip_and_an_empty_folder_reads_as_none() {
        let home = std::env::temp_dir().join(format!("warcry-macros-{}", std::process::id()));
        std::fs::remove_dir_all(&home).ok();
        assert!(drafts(&home).is_empty());
        let wanted = vec![Draft {
            name: "wc_heal".into(),
            body: "/cast [@party1,help,nodead] Healing Touch".into(),
        }];
        save(&home, &wanted).unwrap();
        assert_eq!(drafts(&home), wanted);
        std::fs::remove_dir_all(&home).ok();
    }

    #[test]
    fn a_character_only_counts_when_it_has_a_cache_file() {
        let root = std::env::temp_dir().join(format!("warcry-seats-{}", std::process::id()));
        let account = root.join("_forever_/WTF/Account/1234#1");
        let who = account.join("91/Sky-Warden");
        std::fs::create_dir_all(&who).unwrap();
        std::fs::create_dir_all(account.join("91/Empty-One")).unwrap();
        std::fs::write(who.join(CACHE), SAMPLE).unwrap();
        std::fs::write(account.join(CACHE), SAMPLE).unwrap();
        let found = in_flavor(&root.join("_forever_"));
        let names: Vec<&str> = found.iter().map(|c| c.name.as_str()).collect();
        assert_eq!(names, vec![GENERAL, "Sky-Warden"]);
        assert!(found.iter().all(|c| c.macros == 2));
        assert_eq!(found[1].realm, "91");
        assert_eq!(found[1].flavor, "Forever");
        assert_eq!(found[1].account, "1234#1");
        std::fs::remove_dir_all(root).ok();
    }
}
