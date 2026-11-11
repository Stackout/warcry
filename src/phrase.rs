use anyhow::{Result, ensure};
use std::collections::BTreeSet;

pub const MAX_EXPANSIONS: usize = 4096;

pub fn expand(pattern: &str) -> Result<Vec<String>> {
    let chars: Vec<char> = pattern.chars().collect();
    let (alts, rest) = alternatives(&chars)?;
    ensure!(rest.is_empty(), "unbalanced ']' in phrase {pattern:?}");
    let phrases: BTreeSet<String> = alts
        .iter()
        .map(|p| {
            p.split_whitespace()
                .collect::<Vec<_>>()
                .join(" ")
                .to_lowercase()
        })
        .filter(|p| !p.is_empty())
        .collect();
    ensure!(!phrases.is_empty(), "phrase {pattern:?} expands to nothing");
    Ok(phrases.into_iter().collect())
}

fn alternatives(chars: &[char]) -> Result<(Vec<String>, &[char])> {
    let (first, rest) = sequence(chars)?;
    match rest.first() {
        Some(';') => {
            let (others, rest) = alternatives(&rest[1..])?;
            Ok(([first, others].concat(), rest))
        }
        _ => Ok((first, rest)),
    }
}

fn sequence(chars: &[char]) -> Result<(Vec<String>, &[char])> {
    match chars.first() {
        None | Some(';') | Some(']') => Ok((vec![String::new()], chars)),
        Some('[') => {
            let (group, rest) = alternatives(&chars[1..])?;
            ensure!(rest.first() == Some(&']'), "unclosed '[' in phrase");
            let (tail, rest) = sequence(&rest[1..])?;
            ensure!(
                group.len() * tail.len() <= MAX_EXPANSIONS,
                "phrase has over {MAX_EXPANSIONS} variants"
            );
            let joined = group
                .iter()
                .flat_map(|g| tail.iter().map(move |t| format!("{g} {t}")))
                .collect();
            Ok((joined, rest))
        }
        Some(c) => {
            let (tail, rest) = sequence(&chars[1..])?;
            Ok((tail.into_iter().map(|t| format!("{c}{t}")).collect(), rest))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain() {
        assert_eq!(expand("Cast  Line").unwrap(), vec!["cast line"]);
    }

    #[test]
    fn top_level_alternatives() {
        assert_eq!(
            expand("reel in;pull it in").unwrap(),
            vec!["pull it in", "reel in"]
        );
    }

    #[test]
    fn groups_and_optional() {
        assert_eq!(
            expand("[cast;throw] [the;] line").unwrap(),
            vec!["cast line", "cast the line", "throw line", "throw the line"]
        );
    }

    #[test]
    fn nested() {
        assert_eq!(
            expand("[reel [it;] in;pull]").unwrap(),
            vec!["pull", "reel in", "reel it in"]
        );
    }

    #[test]
    fn errors() {
        assert!(expand("[cast").is_err());
        assert!(expand("cast]").is_err());
        assert!(expand("[;]").is_err());
    }
}
