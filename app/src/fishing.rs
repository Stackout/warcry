use crate::lua::{self, Value};
use serde::Serialize;
use std::path::{Path, PathBuf};

const SAVED: &str = "WarcryBeacon.lua";
const GLOBAL: &str = "WarcryLedgerDB";

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Fish {
    pub name: String,
    pub count: u32,
    pub auction: i64,
    pub vendor: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Spot {
    pub zone: String,
    pub catches: u32,
    pub auction: i64,
}

#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct Session {
    pub started: i64,
    pub seconds: u64,
    pub casts: u32,
    pub catches: u32,
    pub missed: u32,
    pub auction: i64,
    pub vendor: i64,
    pub worth: i64,
    pub per_hour: i64,
    pub fish: Vec<Fish>,
    pub spots: Vec<Spot>,
}

#[derive(Debug, Clone, Serialize, Default, PartialEq)]
pub struct Ledger {
    pub sessions: Vec<Session>,
    pub worth: i64,
    pub catches: u32,
    pub casts: u32,
    pub seconds: u64,
    pub per_hour: i64,
}

fn per_hour(copper: i64, seconds: u64) -> i64 {
    match seconds < 60 {
        true => 0,
        false => (copper as f64 / (seconds as f64 / 3600.0)) as i64,
    }
}

fn fish_of(table: &Value) -> Vec<Fish> {
    let mut all: Vec<Fish> = table
        .entries()
        .iter()
        .map(|(_, row)| Fish {
            name: row.text("name"),
            count: row.number("count") as u32,
            auction: row.number("auction") as i64,
            vendor: row.number("vendor") as i64,
        })
        .filter(|fish| fish.count > 0)
        .collect();
    all.sort_by(|a, b| b.auction.cmp(&a.auction).then(b.count.cmp(&a.count)));
    all
}

fn spots_of(table: &Value) -> Vec<Spot> {
    let mut all: Vec<Spot> = table
        .entries()
        .iter()
        .filter_map(|(key, row)| match key {
            lua::Key::Name(zone) => Some(Spot {
                zone: zone.clone(),
                catches: row.number("catches") as u32,
                auction: row.number("auction") as i64,
            }),
            lua::Key::Index(_) => None,
        })
        .collect();
    all.sort_by_key(|spot| std::cmp::Reverse(spot.auction));
    all
}

fn session_of(row: &Value) -> Session {
    let (casts, catches) = (row.number("casts") as u32, row.number("catches") as u32);
    let (auction, vendor) = (row.number("auction") as i64, row.number("vendor") as i64);
    let worth = auction.max(vendor);
    let seconds = row.number("seconds") as u64;
    Session {
        started: row.number("started") as i64,
        seconds,
        casts,
        catches,
        missed: casts.saturating_sub(catches),
        auction,
        vendor,
        worth,
        per_hour: per_hour(worth, seconds),
        fish: row.get("fish").map(fish_of).unwrap_or_default(),
        spots: row.get("zones").map(spots_of).unwrap_or_default(),
    }
}

pub fn of(source: &str) -> Ledger {
    let Some(db) = lua::parse(source, GLOBAL) else {
        return Ledger::default();
    };
    let mut sessions: Vec<Session> = db
        .get("sessions")
        .map(|list| {
            list.entries()
                .iter()
                .map(|(_, row)| session_of(row))
                .collect()
        })
        .unwrap_or_default();
    sessions.retain(|session| session.casts > 0);
    sessions.sort_by_key(|session| std::cmp::Reverse(session.started));
    let worth = sessions.iter().map(|s| s.worth).sum();
    let seconds = sessions.iter().map(|s| s.seconds).sum();
    Ledger {
        worth,
        catches: sessions.iter().map(|s| s.catches).sum(),
        casts: sessions.iter().map(|s| s.casts).sum(),
        seconds,
        per_hour: per_hour(worth, seconds),
        sessions,
    }
}

fn saved_files(flavor: &Path) -> Vec<PathBuf> {
    let accounts = flavor.join("WTF").join("Account");
    std::fs::read_dir(accounts)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path().join("SavedVariables").join(SAVED))
        .filter(|path| path.is_file())
        .collect()
}

pub fn read(flavor: &Path) -> Ledger {
    let combined: Vec<Ledger> = saved_files(flavor)
        .iter()
        .filter_map(|path| std::fs::read_to_string(path).ok())
        .map(|source| of(&source))
        .collect();
    let mut all: Vec<Session> = combined
        .into_iter()
        .flat_map(|ledger| ledger.sessions)
        .collect();
    all.sort_by_key(|session| std::cmp::Reverse(session.started));
    let worth = all.iter().map(|s| s.worth).sum();
    let seconds = all.iter().map(|s| s.seconds).sum();
    Ledger {
        worth,
        catches: all.iter().map(|s| s.catches).sum(),
        casts: all.iter().map(|s| s.casts).sum(),
        seconds,
        per_hour: per_hour(worth, seconds),
        sessions: all,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"
WarcryLedgerDB = {
	["version"] = 1,
	["sessions"] = {
		{
			["started"] = 1700000000,
			["seconds"] = 3600,
			["casts"] = 50,
			["catches"] = 45,
			["auction"] = 12000,
			["vendor"] = 3000,
			["fish"] = {
				["13756"] = { ["name"] = "Raw Summer Bass", ["count"] = 30, ["auction"] = 9000, ["vendor"] = 2000 },
				["6303"] = { ["name"] = "Raw Glossy Mightfish", ["count"] = 15, ["auction"] = 3000, ["vendor"] = 1000 },
			},
			["zones"] = {
				["Steamwheedle Port"] = { ["catches"] = 40, ["auction"] = 11000 },
				["Tanaris"] = { ["catches"] = 5, ["auction"] = 1000 },
			},
		}, -- [1]
		{
			["started"] = 1700009999,
			["seconds"] = 30,
			["casts"] = 2,
			["catches"] = 2,
			["auction"] = 100,
			["vendor"] = 50,
		}, -- [2]
	},
}
"#;

    #[test]
    fn a_session_is_summarised_from_what_the_addon_saved() {
        let ledger = of(SAMPLE);
        assert_eq!(ledger.sessions.len(), 2);
        let newest = &ledger.sessions[0];
        assert_eq!(newest.started, 1700009999);
        let hour = &ledger.sessions[1];
        assert_eq!(hour.worth, 12000);
        assert_eq!(hour.per_hour, 12000);
        assert_eq!(hour.missed, 5);
    }

    #[test]
    fn vendor_wins_when_it_beats_the_auction_house() {
        let ledger = of(
            r#"WarcryLedgerDB = { ["sessions"] = { { ["casts"] = 1, ["auction"] = 10, ["vendor"] = 900 } } }"#,
        );
        assert_eq!(ledger.sessions[0].worth, 900);
        assert_eq!(ledger.worth, 900);
    }

    #[test]
    fn fish_are_ranked_by_what_they_are_worth() {
        let fish = &of(SAMPLE).sessions[1].fish;
        assert_eq!(fish[0].name, "Raw Summer Bass");
        assert_eq!(fish[1].name, "Raw Glossy Mightfish");
        assert_eq!(fish.len(), 2);
    }

    #[test]
    fn spots_are_ranked_so_the_best_water_is_first() {
        let spots = &of(SAMPLE).sessions[1].spots;
        assert_eq!(spots[0].zone, "Steamwheedle Port");
        assert_eq!(spots[0].catches, 40);
        assert_eq!(spots[1].zone, "Tanaris");
    }

    #[test]
    fn a_short_session_reports_no_hourly_rate_rather_than_a_wild_one() {
        assert_eq!(of(SAMPLE).sessions[0].per_hour, 0);
    }

    #[test]
    fn a_session_with_no_casts_is_not_shown() {
        let ledger =
            of(r#"WarcryLedgerDB = { ["sessions"] = { { ["casts"] = 0 }, { ["casts"] = 3 } } }"#);
        assert_eq!(ledger.sessions.len(), 1);
        assert_eq!(ledger.casts, 3);
    }

    #[test]
    fn nothing_saved_yet_is_an_empty_ledger() {
        assert_eq!(of(""), Ledger::default());
        assert_eq!(of("WarcryLedgerDB = {}"), Ledger::default());
        assert_eq!(read(Path::new("/nonexistent/wow")), Ledger::default());
    }

    #[test]
    fn totals_add_up_across_sessions() {
        let ledger = of(SAMPLE);
        assert_eq!(ledger.casts, 52);
        assert_eq!(ledger.catches, 47);
        assert_eq!(ledger.worth, 12100);
        assert_eq!(ledger.seconds, 3630);
    }
}
