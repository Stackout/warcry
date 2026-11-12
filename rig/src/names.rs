use std::collections::HashMap;
use std::path::Path;
use warcry_ears::tone::identity_hash;

pub struct Names {
    by_identity: HashMap<u16, Vec<String>>,
}

impl Names {
    pub fn scan(flavor: &Path) -> Names {
        let mut by_identity: HashMap<u16, Vec<String>> = HashMap::new();
        for character in characters(flavor) {
            for form in forms(&character) {
                let bucket = by_identity.entry(identity_hash(&form)).or_default();
                if !bucket.contains(&character) {
                    bucket.push(character.clone());
                }
            }
        }
        Names { by_identity }
    }

    pub fn label(&self, identity: u16) -> Option<&str> {
        match self.by_identity.get(&identity)?.as_slice() {
            [only] => Some(only),
            _ => None,
        }
    }

    pub fn known(&self) -> usize {
        self.by_identity
            .values()
            .flat_map(|names| names.iter())
            .collect::<std::collections::HashSet<_>>()
            .len()
    }
}

fn forms(character: &str) -> Vec<String> {
    let bare = character.split('-').next().unwrap_or(character);
    match bare == character {
        true => vec![character.to_string()],
        false => vec![character.to_string(), bare.to_string()],
    }
}

fn characters(flavor: &Path) -> Vec<String> {
    let accounts = flavor.join("WTF").join("Account");
    subdirs(&accounts)
        .iter()
        .flat_map(|account| subdirs(account))
        .flat_map(|realm| subdirs(&realm))
        .filter_map(|character| {
            character
                .file_name()
                .map(|name| name.to_string_lossy().to_string())
        })
        .filter(|name| name != "SavedVariables")
        .collect()
}

fn subdirs(path: &Path) -> Vec<std::path::PathBuf> {
    std::fs::read_dir(path)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.is_dir())
        .filter(|path| {
            path.file_name()
                .is_some_and(|name| name != "SavedVariables")
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(characters: &[&str]) -> tempdir::Dir {
        let dir = tempdir::Dir::new();
        let realm = dir
            .path()
            .join("WTF")
            .join("Account")
            .join("1234#1")
            .join("91");
        std::fs::create_dir_all(&realm).unwrap();
        std::fs::create_dir_all(realm.parent().unwrap().join("SavedVariables")).unwrap();
        for character in characters {
            std::fs::create_dir_all(realm.join(character)).unwrap();
        }
        dir
    }

    #[test]
    fn resolves_a_character_by_its_full_folder_name() {
        let dir = fixture(&["Death-Tailor", "Hawk-Strider"]);
        let names = Names::scan(dir.path());
        assert_eq!(
            names.label(identity_hash("Death-Tailor")),
            Some("Death-Tailor")
        );
        assert_eq!(names.known(), 2);
    }

    #[test]
    fn resolves_a_character_by_the_name_before_the_realm_suffix() {
        let dir = fixture(&["Death-Tailor"]);
        let names = Names::scan(dir.path());
        assert_eq!(names.label(identity_hash("Death")), Some("Death-Tailor"));
    }

    #[test]
    fn an_ambiguous_identity_resolves_to_nothing() {
        let dir = fixture(&["Vex-Warden", "Vex-Stormgale"]);
        let names = Names::scan(dir.path());
        assert_eq!(names.label(identity_hash("Vex")), None);
        assert_eq!(names.label(identity_hash("Vex-Warden")), Some("Vex-Warden"));
    }

    #[test]
    fn saved_variables_is_not_a_character() {
        let dir = fixture(&[]);
        let names = Names::scan(dir.path());
        assert_eq!(names.known(), 0);
    }

    #[test]
    fn a_missing_install_is_empty_rather_than_an_error() {
        let names = Names::scan(Path::new("/nonexistent/wow/flavor"));
        assert_eq!(names.known(), 0);
        assert_eq!(names.label(0), None);
    }

    mod tempdir {
        pub struct Dir(std::path::PathBuf);

        impl Dir {
            pub fn new() -> Dir {
                let path = std::env::temp_dir().join(format!(
                    "warcry-names-{}-{:?}",
                    std::process::id(),
                    std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap()
                        .as_nanos()
                ));
                std::fs::create_dir_all(&path).unwrap();
                Dir(path)
            }

            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                std::fs::remove_dir_all(&self.0).ok();
            }
        }
    }
}
