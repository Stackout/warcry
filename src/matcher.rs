use crate::phrase;
use crate::profile::Profile;
use anyhow::{Context, Result};

#[derive(Debug, Clone)]
struct Entry {
    command: usize,
    phrase: String,
    words: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Match {
    pub command: usize,
    pub phrase: String,
    pub score: f64,
}

#[derive(Debug, Clone, Default)]
pub struct Matcher {
    entries: Vec<Entry>,
}

impl Matcher {
    pub fn build(profile: &Profile) -> Result<Self> {
        let entries = profile
            .commands
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.phrase.as_ref().map(|p| (i, c, p)))
            .map(|(i, c, p)| {
                phrase::expand(p)
                    .with_context(|| format!("command {:?}", c.display_name()))
                    .map(|ps| ps.into_iter().map(move |p| (i, normalize(&p))))
            })
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .flatten()
            .filter(|(_, p)| !p.is_empty())
            .map(|(command, phrase)| Entry {
                command,
                words: phrase.split(' ').count(),
                phrase,
            })
            .collect();
        Ok(Self { entries })
    }

    pub fn phrases_for(&self, command: usize) -> Vec<&str> {
        self.entries
            .iter()
            .filter(|e| e.command == command)
            .map(|e| e.phrase.as_str())
            .collect()
    }

    pub fn best(&self, text: &str, allowed: impl Fn(usize) -> bool) -> Option<Match> {
        let t = normalize(text);
        let words: Vec<&str> = t.split(' ').filter(|w| !w.is_empty()).collect();
        self.entries
            .iter()
            .filter(|e| allowed(e.command) && !words.is_empty())
            .map(|e| (score(&t, &words, e), e))
            .max_by(|(a, ea), (b, eb)| a.total_cmp(b).then(ea.words.cmp(&eb.words)))
            .map(|(score, e)| Match {
                command: e.command,
                phrase: e.phrase.clone(),
                score,
            })
    }

    pub fn prompt(&self, max_chars: usize) -> String {
        self.entries
            .iter()
            .map(|e| e.phrase.as_str())
            .scan(0usize, |len, p| {
                *len += p.len() + 2;
                (*len <= max_chars).then_some(p)
            })
            .collect::<Vec<_>>()
            .join(", ")
    }
}

fn score(text: &str, words: &[&str], e: &Entry) -> f64 {
    let whole = strsim::normalized_levenshtein(text, &e.phrase);
    let inner = words
        .windows(e.words)
        .filter(|_| words.len() > e.words)
        .map(|w| strsim::normalized_levenshtein(&w.join(" "), &e.phrase) * 0.95)
        .fold(0.0, f64::max);
    whole.max(inner)
}

pub fn normalize(text: &str) -> String {
    let (_, cleaned) = text
        .chars()
        .fold((0i32, String::new()), |(depth, mut out), c| match c {
            '[' | '(' => (depth + 1, out),
            ']' | ')' => ((depth - 1).max(0), out),
            _ if depth > 0 => (depth, out),
            '\'' | '’' => (depth, out),
            c if c.is_alphanumeric() => {
                out.extend(c.to_lowercase());
                (depth, out)
            }
            _ => {
                out.push(' ');
                (depth, out)
            }
        });
    cleaned
        .split_whitespace()
        .map(|w| number_word(w).unwrap_or(w))
        .collect::<Vec<_>>()
        .join(" ")
}

fn number_word(w: &str) -> Option<&'static str> {
    const WORDS: [&str; 21] = [
        "zero",
        "one",
        "two",
        "three",
        "four",
        "five",
        "six",
        "seven",
        "eight",
        "nine",
        "ten",
        "eleven",
        "twelve",
        "thirteen",
        "fourteen",
        "fifteen",
        "sixteen",
        "seventeen",
        "eighteen",
        "nineteen",
        "twenty",
    ];
    w.parse::<usize>().ok().and_then(|n| WORDS.get(n).copied())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRC: &str = r#"
        [[command]]
        phrase = "[cast;throw] [the;] line"
        [[command]]
        phrase = "reel in"
        [[command]]
        phrase = "stop;stop reeling"
        [[command]]
        phrase = "use slot 2"
    "#;

    fn matcher() -> Matcher {
        Matcher::build(&Profile::parse(SRC).unwrap()).unwrap()
    }

    #[test]
    fn normalize_strips_noise() {
        assert_eq!(normalize("  Cast the line! [BLANK_AUDIO]"), "cast the line");
        assert_eq!(normalize("(music) Reel in."), "reel in");
        assert_eq!(normalize("Don't stop"), "dont stop");
        assert_eq!(normalize("slot 2"), "slot two");
    }

    #[test]
    fn exact_and_fuzzy() {
        let m = matcher();
        let r = m.best("Cast the line.", |_| true).unwrap();
        assert_eq!((r.command, r.score), (0, 1.0));
        let r = m.best("real in", |_| true).unwrap();
        assert!(r.command == 1 && r.score > 0.8);
        let r = m.best("okay throw line please", |_| true).unwrap();
        assert!(r.command == 0 && r.score > 0.9);
        assert_eq!(m.best("use slot two", |_| true).unwrap().score, 1.0);
        assert!(m.best("", |_| true).is_none());
    }

    #[test]
    fn longer_phrase_wins_ties() {
        assert_eq!(
            matcher().best("stop reeling", |_| true).unwrap().phrase,
            "stop reeling"
        );
    }

    #[test]
    fn filter_respected() {
        assert_ne!(matcher().best("reel in", |c| c != 1).unwrap().command, 1);
    }

    #[test]
    fn prompt_is_bounded() {
        assert!(matcher().prompt(20).len() <= 20);
    }
}
