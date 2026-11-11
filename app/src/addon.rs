use anyhow::{Context, Result, bail};
use std::path::{Path, PathBuf};

const FOLDER: &str = "WarcryBeacon";
const TOC: &str = include_str!("../../addon/WarcryBeacon/WarcryBeacon.toc");
const LUA: &str = include_str!("../../addon/WarcryBeacon/WarcryBeacon.lua");

const TONES: [(&str, &[u8]); 8] = [
    (
        "t0.ogg",
        include_bytes!("../../addon/WarcryBeacon/tones/t0.ogg"),
    ),
    (
        "t1.ogg",
        include_bytes!("../../addon/WarcryBeacon/tones/t1.ogg"),
    ),
    (
        "t2.ogg",
        include_bytes!("../../addon/WarcryBeacon/tones/t2.ogg"),
    ),
    (
        "t3.ogg",
        include_bytes!("../../addon/WarcryBeacon/tones/t3.ogg"),
    ),
    (
        "t4.ogg",
        include_bytes!("../../addon/WarcryBeacon/tones/t4.ogg"),
    ),
    (
        "t5.ogg",
        include_bytes!("../../addon/WarcryBeacon/tones/t5.ogg"),
    ),
    (
        "t6.ogg",
        include_bytes!("../../addon/WarcryBeacon/tones/t6.ogg"),
    ),
    (
        "t7.ogg",
        include_bytes!("../../addon/WarcryBeacon/tones/t7.ogg"),
    ),
];

pub fn version() -> String {
    TOC.lines()
        .find_map(|line| line.strip_prefix("## Version:"))
        .map(|value| value.trim().to_string())
        .unwrap_or_else(|| "?".into())
}

pub fn roots() -> Vec<PathBuf> {
    let guesses = [
        "/Applications/World of Warcraft",
        "C:\\Program Files (x86)\\World of Warcraft",
        "C:\\Program Files\\World of Warcraft",
        "D:\\World of Warcraft",
    ];
    guesses
        .iter()
        .map(PathBuf::from)
        .filter(|root| root.is_dir())
        .collect()
}

pub fn flavors(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with('_') && name.ends_with('_'))
        })
        .collect()
}

pub fn addons(flavor: &Path) -> PathBuf {
    flavor.join("Interface").join("AddOns").join(FOLDER)
}

pub fn installed(flavor: &Path) -> Option<String> {
    std::fs::read_to_string(addons(flavor).join("WarcryBeacon.toc"))
        .ok()?
        .lines()
        .find_map(|line| line.strip_prefix("## Version:"))
        .map(|value| value.trim().to_string())
}

pub fn install(flavor: &Path) -> Result<usize> {
    if !flavor.is_dir() {
        bail!("{} is not a World of Warcraft folder", flavor.display());
    }
    let target = addons(flavor);
    let tones = target.join("tones");
    std::fs::create_dir_all(&tones).with_context(|| format!("creating {}", tones.display()))?;
    std::fs::write(target.join("WarcryBeacon.toc"), TOC)?;
    std::fs::write(target.join("WarcryBeacon.lua"), LUA)?;
    TONES
        .iter()
        .try_for_each(|(name, bytes)| std::fs::write(tones.join(name), bytes))
        .with_context(|| format!("writing tones into {}", tones.display()))?;
    Ok(2 + TONES.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_bundled_addon_carries_a_version() {
        assert_ne!(version(), "?");
        assert!(!LUA.is_empty());
        assert_eq!(TONES.len(), 8);
        assert!(TONES.iter().all(|(_, bytes)| bytes.len() > 1000));
    }

    #[test]
    fn installing_writes_every_file_and_reports_its_version() {
        let flavor =
            std::env::temp_dir().join(format!("warcry-addon-{}/_test_", std::process::id()));
        std::fs::create_dir_all(&flavor).unwrap();
        assert_eq!(installed(&flavor), None);
        assert_eq!(install(&flavor).unwrap(), 10);
        assert_eq!(installed(&flavor), Some(version()));
        assert!(addons(&flavor).join("tones").join("t7.ogg").exists());
        std::fs::remove_dir_all(flavor.parent().unwrap()).ok();
    }

    #[test]
    fn installing_somewhere_that_is_not_a_game_fails() {
        assert!(install(Path::new("/nope/not/a/game")).is_err());
    }

    #[test]
    fn flavors_are_only_underscored_folders() {
        let root = std::env::temp_dir().join(format!("warcry-flavors-{}", std::process::id()));
        std::fs::create_dir_all(root.join("_retail_")).unwrap();
        std::fs::create_dir_all(root.join("_forever_")).unwrap();
        std::fs::create_dir_all(root.join("Logs")).unwrap();
        let mut found: Vec<String> = flavors(&root)
            .iter()
            .filter_map(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .collect();
        found.sort();
        assert_eq!(found, vec!["_forever_", "_retail_"]);
        std::fs::remove_dir_all(root).ok();
    }
}
