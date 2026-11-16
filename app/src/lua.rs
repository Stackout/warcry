#[derive(Debug, Clone, PartialEq)]
pub enum Key {
    Name(String),
    Index(usize),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Table(Vec<(Key, Value)>),
    Text(String),
    Number(f64),
    Truth(bool),
    Nothing,
}

impl Value {
    pub fn get(&self, name: &str) -> Option<&Value> {
        self.entries()
            .iter()
            .find(|(key, _)| *key == Key::Name(name.to_string()))
            .map(|(_, value)| value)
    }

    pub fn entries(&self) -> &[(Key, Value)] {
        match self {
            Value::Table(rows) => rows,
            _ => &[],
        }
    }

    pub fn number(&self, name: &str) -> f64 {
        match self.get(name) {
            Some(Value::Number(n)) => *n,
            _ => 0.0,
        }
    }

    pub fn text(&self, name: &str) -> String {
        match self.get(name) {
            Some(Value::Text(t)) => t.clone(),
            _ => String::new(),
        }
    }
}

struct Scan<'a> {
    bytes: &'a [u8],
    at: usize,
}

impl<'a> Scan<'a> {
    fn new(source: &'a str) -> Scan<'a> {
        Scan {
            bytes: source.as_bytes(),
            at: 0,
        }
    }

    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.at).copied()
    }

    fn skip(&mut self) {
        while let Some(byte) = self.peek() {
            if byte.is_ascii_whitespace() {
                self.at += 1;
                continue;
            }
            if byte == b'-' && self.bytes.get(self.at + 1) == Some(&b'-') {
                while self.peek().is_some_and(|b| b != b'\n') {
                    self.at += 1;
                }
                continue;
            }
            return;
        }
    }

    fn eat(&mut self, byte: u8) -> bool {
        self.skip();
        let found = self.peek() == Some(byte);
        self.at += found as usize;
        found
    }

    fn string(&mut self) -> String {
        let quote = self.bytes[self.at];
        self.at += 1;
        let mut out = String::new();
        while let Some(byte) = self.peek() {
            self.at += 1;
            if byte == quote {
                break;
            }
            if byte != b'\\' {
                out.push(byte as char);
                continue;
            }
            let escaped = self.peek().unwrap_or(b'\\');
            self.at += 1;
            out.push(match escaped {
                b'n' => '\n',
                b't' => '\t',
                other => other as char,
            });
        }
        out
    }

    fn word(&mut self) -> String {
        let start = self.at;
        while self
            .peek()
            .is_some_and(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'.' || b == b'-')
        {
            self.at += 1;
        }
        String::from_utf8_lossy(&self.bytes[start..self.at]).to_string()
    }

    fn value(&mut self) -> Value {
        self.skip();
        match self.peek() {
            None => Value::Nothing,
            Some(b'{') => self.table(),
            Some(b'"') | Some(b'\'') => Value::Text(self.string()),
            Some(_) => self.scalar(),
        }
    }

    fn scalar(&mut self) -> Value {
        let word = self.word();
        match word.as_str() {
            "true" => Value::Truth(true),
            "false" => Value::Truth(false),
            "nil" | "" => Value::Nothing,
            other => other
                .parse::<f64>()
                .map(Value::Number)
                .unwrap_or(Value::Text(other.to_string())),
        }
    }

    fn key(&mut self) -> Option<Key> {
        self.skip();
        if !self.eat(b'[') {
            return None;
        }
        self.skip();
        let key = match self.peek() {
            Some(b'"') | Some(b'\'') => Key::Name(self.string()),
            _ => match self.scalar() {
                Value::Number(n) => Key::Index(n as usize),
                Value::Text(t) => Key::Name(t),
                _ => Key::Index(0),
            },
        };
        self.eat(b']');
        self.eat(b'=').then_some(key)
    }

    fn table(&mut self) -> Value {
        self.at += 1;
        let mut rows = Vec::new();
        let mut next = 1;
        loop {
            self.skip();
            match self.peek() {
                None => break,
                Some(b'}') => {
                    self.at += 1;
                    break;
                }
                Some(b',') | Some(b';') => {
                    self.at += 1;
                    continue;
                }
                Some(_) => {}
            }
            let mark = self.at;
            let key = self.key().unwrap_or_else(|| {
                self.at = mark;
                let key = Key::Index(next);
                next += 1;
                key
            });
            rows.push((key, self.value()));
        }
        Value::Table(rows)
    }
}

pub fn parse(source: &str, global: &str) -> Option<Value> {
    let start = source.find(global)?;
    let rest = &source[start + global.len()..];
    let equals = rest.find('=')?;
    let mut scan = Scan::new(&rest[equals + 1..]);
    Some(scan.value())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_saved_variables_table_the_way_the_game_writes_it() {
        let source = r#"
WarcryLedgerDB = {
	["version"] = 1,
	["sessions"] = {
		{
			["casts"] = 47,
			["auction"] = 12430,
			["fish"] = {
				["13756"] = {
					["name"] = "Raw Summer Bass",
					["count"] = 18,
				},
			},
		}, -- [1]
	},
}
"#;
        let db = parse(source, "WarcryLedgerDB").unwrap();
        assert_eq!(db.number("version"), 1.0);
        let sessions = db.get("sessions").unwrap();
        assert_eq!(sessions.entries().len(), 1);
        let first = &sessions.entries()[0].1;
        assert_eq!(first.number("casts"), 47.0);
        assert_eq!(first.number("auction"), 12430.0);
        let fish = first.get("fish").unwrap();
        assert_eq!(fish.entries()[0].0, Key::Name("13756".into()));
        assert_eq!(fish.entries()[0].1.text("name"), "Raw Summer Bass");
        assert_eq!(fish.entries()[0].1.number("count"), 18.0);
    }

    #[test]
    fn a_comment_after_a_row_is_not_part_of_the_next_one() {
        let db = parse("T = { [1] = 5, -- [1]\n [2] = 6 }", "T").unwrap();
        assert_eq!(db.entries().len(), 2);
        assert_eq!(db.entries()[1].1, Value::Number(6.0));
    }

    #[test]
    fn a_list_without_keys_is_numbered_from_one() {
        let db = parse(r#"T = { "a", "b", "c" }"#, "T").unwrap();
        assert_eq!(db.entries()[0].0, Key::Index(1));
        assert_eq!(db.entries()[2].0, Key::Index(3));
        assert_eq!(db.entries()[2].1, Value::Text("c".into()));
    }

    #[test]
    fn escapes_and_floats_and_booleans_survive() {
        let db = parse(
            r#"T = { ["a"] = "say \"hi\"", ["b"] = 1.5, ["c"] = true }"#,
            "T",
        )
        .unwrap();
        assert_eq!(db.text("a"), "say \"hi\"");
        assert_eq!(db.number("b"), 1.5);
        assert_eq!(db.get("c"), Some(&Value::Truth(true)));
    }

    #[test]
    fn a_missing_global_is_none_rather_than_a_panic() {
        assert_eq!(parse("Other = {}", "WarcryLedgerDB"), None);
        assert_eq!(parse("", "WarcryLedgerDB"), None);
    }

    #[test]
    fn a_truncated_table_does_not_hang_or_panic() {
        let db = parse(r#"T = { ["a"] = { ["b"] = 1"#, "T").unwrap();
        assert_eq!(db.get("a").unwrap().number("b"), 1.0);
    }
}
