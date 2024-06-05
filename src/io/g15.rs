#![allow(non_upper_case_globals)]
#![allow(unused_variables)]
#![allow(clippy::missing_errors_doc)]

use anyhow::Result;
use once_cell::sync::Lazy;
use regex::Regex;
use steamid_ng::SteamID;

use crate::player::Team;

/// `m_iAmmo[1] integer (32)` --> capture groups: `(player idx)` `(ammo count)`
pub const REGEX_I_AMMO: &str = r"^m_iAmmo\[(\d+)\]\s+integer\s+\((\d+)\)$";
/// `m_szName[1] string (Lust)` --> capture groups: `(player idx)` `(string
/// name)`
pub const REGEX_SZ_NAME: &str = r"^m_szName\[(\d+)\]\s+string\s+\((.+)\)$";
/// `m_iPing[2] integer (15)` --> capture groups: `(player idx)` `(ping in ms)`
pub const REGEX_I_PING: &str = r"^m_iPing\[(\d+)\]\s+integer\s+\((\d+)\)$";
/// `m_iScore[4] integer (7)` --> capture groups: `(player idx)` `(score (num of
/// kills))`
pub const REGEX_I_SCORE: &str = r"^m_iScore\[(\d+)\]\s+integer\s+\((\d+)\)$";
/// `m_iDeaths[17] integer (4)` --> capture groups: `(player idx)` `(deaths)`
pub const REGEX_I_DEATHS: &str = r"^m_iDeaths\[(\d+)\]\s+integer\s+\((\d+)\)$";
/// `m_bConnected[0] bool (false)` --> capture groups:  `(player idx)`
/// `(connected status (true/false))`
pub const REGEX_B_CONNECTED: &str = r"^m_bConnected\[(\d+)\]\s+bool\s+\((false|true)\)$";
/// `m_iTeam[18] integer (3)` --> capture groups: `(player idx)` `(team
/// (0=unconnected, 1=spectator, 2=blu, 3=red))`
pub const REGEX_I_TEAM: &str = r"^m_iTeam\[(\d+)\]\s+integer\s+\(([0-3])\)$";
/// `m_bAlive[0] bool (false)` --> capture groups: `(player idx)` `(alive status
/// (true/false))`
pub const REGEX_B_ALIVE: &str = r"^m_bAlive\[(\d+)\]\s+bool\s+\((false|true)\)$";
/// `m_iHealth[3] integer (125)` --> capture groups: `(player idx)` `(current
/// health)`
pub const REGEX_I_HEALTH: &str = r"^m_iHealth\[(\d+)\]\s+integer\s+\((\d+)\)$";
/// `m_iAccountID[3] integer (1505713148)` --> capture groups: `(player idx)`
/// `(variable component of a steamID3 ([U:1:1505713148]))`
pub const REGEX_I_SID3: &str = r"^m_iAccountID\[(\d+)\]\s+integer\s+\((\d{4,})\)$";
/// `m_bValid[24] bool (true)` --> capture groups: `(player idx)` `(valid status
/// (true/false))` I'm not sure what valid means - perhaps to with whether this
/// is a current/in-game player idx and to display this row in the scoreboard or
/// not? Can someone look pls
pub const REGEX_B_VALID: &str = r"^m_bValid\[(\d+)\]\s+bool\s+\((false|true)\)$";
/// `m_iUserID[2] integer (68)` --> capture groups: `(player idx)` `(server id
/// (as it appears in status))`
pub const REGEX_I_USERID: &str = r"^m_iUserID\[(\d+)\]\s+integer\s+\((\d+)\)$";

static PATTERNS: [Lazy<(Regex, G15Field)>; 12] = [
    Lazy::new(|| {
        (
            Regex::new(REGEX_I_AMMO).expect("Regex compiler error"),
            G15Field::Ammo,
        )
    }),
    Lazy::new(|| {
        (
            Regex::new(REGEX_SZ_NAME).expect("Regex compiler error"),
            G15Field::Name,
        )
    }),
    Lazy::new(|| {
        (
            Regex::new(REGEX_I_PING).expect("Regex compiler error"),
            G15Field::Ping,
        )
    }),
    Lazy::new(|| {
        (
            Regex::new(REGEX_I_SCORE).expect("Regex compiler error"),
            G15Field::Score,
        )
    }),
    Lazy::new(|| {
        (
            Regex::new(REGEX_I_DEATHS).expect("Regex compiler error"),
            G15Field::Deaths,
        )
    }),
    Lazy::new(|| {
        (
            Regex::new(REGEX_B_CONNECTED).expect("Regex compiler error"),
            G15Field::Connected,
        )
    }),
    Lazy::new(|| {
        (
            Regex::new(REGEX_I_TEAM).expect("Regex compiler error"),
            G15Field::Team,
        )
    }),
    Lazy::new(|| {
        (
            Regex::new(REGEX_B_ALIVE).expect("Regex compiler error"),
            G15Field::Alive,
        )
    }),
    Lazy::new(|| {
        (
            Regex::new(REGEX_I_HEALTH).expect("Regex compiler error"),
            G15Field::Health,
        )
    }),
    Lazy::new(|| {
        (
            Regex::new(REGEX_I_SID3).expect("Regex compiler error"),
            G15Field::SteamID3,
        )
    }),
    Lazy::new(|| {
        (
            Regex::new(REGEX_B_VALID).expect("Regex compiler error"),
            G15Field::Valid,
        )
    }),
    Lazy::new(|| {
        (
            Regex::new(REGEX_I_USERID).expect("Regex compiler error"),
            G15Field::UserID,
        )
    }),
];

enum G15Field {
    Ammo,
    Name,
    Ping,
    Score,
    Deaths,
    Connected,
    Team,
    Alive,
    Health,
    SteamID3,
    Valid,
    UserID,
}

enum G15FieldValue {
    U32(u32),
    String(String),
    Bool(bool),
    Team(Option<Team>),
    SteamID(SteamID),
}

impl G15FieldValue {
    fn from_str(type_definer: &G15Field, captured_value: &str) -> Result<G15FieldValue> {
        Ok(match type_definer {
            G15Field::Ammo
            | G15Field::Score
            | G15Field::Ping
            | G15Field::Deaths
            | G15Field::Health => G15FieldValue::U32(captured_value.parse()?),
            G15Field::Name | G15Field::UserID => G15FieldValue::String(captured_value.parse()?),
            G15Field::Connected | G15Field::Alive | G15Field::Valid => {
                G15FieldValue::Bool(matches!(captured_value, "true"))
            }
            G15Field::Team => G15FieldValue::Team(Team::try_from(captured_value).ok()),
            G15Field::SteamID3 => {
                G15FieldValue::SteamID(SteamID::from_steam3(&format!("[U:1:{captured_value}]"))?)
            }
        })
    }
}

struct G15Extraction<'a> {
    field_type: &'a G15Field,
    slot_idx: usize,
    value: G15FieldValue,
}

impl G15Extraction<'_> {
    fn from_string(g15_line: &str) -> Option<G15Extraction> {
        PATTERNS.iter().find_map(|lazy_cell| {
            let caps = lazy_cell.0.captures(g15_line.trim())?;
            let val = G15FieldValue::from_str(&lazy_cell.1, &caps[2]).ok()?;
            Some(G15Extraction {
                field_type: &lazy_cell.1,
                slot_idx: caps[1].parse().ok()?,
                value: val,
            })
        })
    }
}

/// July 25, 2023 TF2 update updated the maximum supported number of players in
/// a single server instance to 100, from 32. This is unlikely to ever change
/// again, so rather than dynamically handling `g15_dumpplayer` logs that
/// can contain any number of array entries, we simply hard-code the current
/// maximum amount of players you will EVER find in a game.
const MAX_POSSIBLE_PLAYERS: u8 = 100;

/// The `G15Player` struct contains all the data per player searched.
/// Each of the elements may be None, as the parsing could fail on a particular
/// line, leaving the value undefined. We still want the rest of the useful data
/// though
#[derive(Debug, Clone)]
#[allow(clippy::module_name_repetitions)]
pub struct G15Player {
    pub name: Option<String>,     // eg "Lilith"
    pub ping: Option<u32>,        // eg 21
    pub score: Option<u32>,       // eg 16
    pub deaths: Option<u32>,      // eg 5
    pub steamid: Option<SteamID>, // eg [U:1:111216987]
    pub team: Option<Team>,       // eg 3
    pub health: Option<u32>,      // eg 125
    pub ammo: Option<u32>,        // eg 6
    pub connected: Option<bool>,  // eg true
    pub valid: Option<bool>,      // eg true
    pub alive: Option<bool>,      // eg true
    pub userid: Option<String>,   // eg "301"
}
impl G15Player {
    const fn new() -> Self {
        Self {
            name: None,
            ping: None,
            score: None,
            deaths: None,
            steamid: None,
            team: None,
            health: None,
            ammo: None,
            alive: None,
            connected: None,
            userid: None,
            valid: None,
        }
    }

    fn apply_from_field_type(&mut self, field_type: &G15Field, field_value: G15FieldValue) {
        match field_type {
            G15Field::Ammo => {
                if let G15FieldValue::U32(val) = field_value {
                    self.ammo = Some(val);
                }
            }
            G15Field::Name => {
                if let G15FieldValue::String(val) = field_value {
                    self.name = Some(val);
                }
            }
            G15Field::Ping => {
                if let G15FieldValue::U32(val) = field_value {
                    self.ping = Some(val);
                }
            }
            G15Field::Score => {
                if let G15FieldValue::U32(val) = field_value {
                    self.score = Some(val);
                }
            }
            G15Field::Deaths => {
                if let G15FieldValue::U32(val) = field_value {
                    self.deaths = Some(val);
                }
            }
            G15Field::Connected => {
                if let G15FieldValue::Bool(val) = field_value {
                    self.connected = Some(val);
                }
            }
            G15Field::Team => {
                if let G15FieldValue::Team(val) = field_value {
                    self.team = val;
                }
            }
            G15Field::Alive => {
                if let G15FieldValue::Bool(val) = field_value {
                    self.alive = Some(val);
                }
            }
            G15Field::Health => {
                if let G15FieldValue::U32(val) = field_value {
                    self.health = Some(val);
                }
            }
            G15Field::SteamID3 => {
                if let G15FieldValue::SteamID(val) = field_value {
                    self.steamid = Some(val);
                }
            }
            G15Field::Valid => {
                if let G15FieldValue::Bool(val) = field_value {
                    self.valid = Some(val);
                }
            }
            G15Field::UserID => {
                if let G15FieldValue::String(val) = field_value {
                    self.userid = Some(val);
                }
            }
        }
    }
}

impl Default for G15Player {
    fn default() -> Self {
        Self::new()
    }
}

/// The `G15Parser` struct contains a vector of compiled Regex automata to parse
/// strings out of the `g15_dumpplayer` console output.
/// It implements a `parse_g15` fn to use these to parse useful player data from
/// the output.
pub struct Parser;

impl Parser {
    /// Parse a `g15_dumpplayer` string via Regex search.
    /// We only extract useful data here, so drop most data.
    #[must_use]
    pub fn parse_g15(g15_log: &str) -> Vec<G15Player> {
        let mut players: Vec<G15Player> =
            vec![G15Player::new(); (MAX_POSSIBLE_PLAYERS + 2) as usize];
        let lines = g15_log.split('\n');

        lines
            .filter_map(G15Extraction::from_string)
            .for_each(|extraction| {
                players[extraction.slot_idx]
                    .apply_from_field_type(extraction.field_type, extraction.value);
            });
        // Only get players who are "valid" and have a non-zero userid
        players.retain(|x| x.valid.unwrap_or(false) && x.userid.as_deref().unwrap_or("0") != "0");
        players
    }
}
