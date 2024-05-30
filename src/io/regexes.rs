#![allow(non_upper_case_globals)]
#![allow(unused_variables)]

use anyhow::{Context, Ok, Result};
use regex::Captures;
use serde::{Deserialize, Serialize};
use steamid_ng::SteamID;

use crate::player::PlayerState;

/*
    Useful commands:
        status
        g15_dumpplayer

        callvote kick <userid>
        vote option<1/2> // Can't really use

*/

pub const REGEX_HOSTNAME: &str = r"^hostname: (.*)$";
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Hostname(pub String);
impl Hostname {
    #[must_use]
    pub fn parse(caps: &Captures) -> Self {
        Self(caps[1].into())
    }
}

pub const REGEX_IP: &str = r"^udp/ip  : (.*)$";
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerIP(pub String);
impl ServerIP {
    #[must_use]
    pub fn parse(caps: &Captures) -> Self {
        Self(caps[1].into())
    }
}

pub const REGEX_MAP: &str = r"^map     : (.+) at: .*$";
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Map(pub String);
impl Map {
    #[must_use]
    pub fn parse(caps: &Captures) -> Self {
        Self(caps[1].into())
    }
}

pub const REGEX_PLAYERCOUNT: &str = r"^players : (\d+) humans, (\d+) bots \((\d+) max\)$";
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlayerCount {
    pub players: u32,
    pub bots: u32,
    pub max: u32,
}

impl PlayerCount {
    #[must_use]
    pub fn parse(caps: &Captures) -> Self {
        Self {
            // Regex should guarantee the input is valid, but in the ridiculous case there's an
            // invalid number I would prefer it has the incorrect value (0) than crash.
            players: caps[1].parse().unwrap_or(0),
            bots: caps[2].parse().unwrap_or(0),
            max: caps[3].parse().unwrap_or(0),
        }
    }
}

/// Player killed someone
/// Matches:
///    0: Killer
///    1: Victim
///    2: Weapon
///    3: Crit?
pub const REGEX_KILL: &str = r"^(.*)\skilled\s(.*)\swith\s(.*)\.(\s\(crit\))?$";
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct PlayerKill {
    pub killer_name: String,
    pub killer_steamid: Option<String>,
    pub victim_name: String,
    pub victim_steamid: Option<String>,
    pub weapon: String,
    pub crit: bool,
}

impl PlayerKill {
    #[must_use]
    pub fn parse(caps: &Captures) -> Self {
        Self {
            killer_name: caps[1].into(),
            killer_steamid: None,
            victim_name: caps[2].into(),
            victim_steamid: None,
            weapon: caps[3].into(),
            crit: caps.get(4).is_some(),
        }
    }

    pub fn set_steam_id_killer(&mut self, id: SteamID) {
        self.killer_steamid = Some(format!("{}", u64::from(id)));
    }

    pub fn set_steam_id_victim(&mut self, id: SteamID) {
        self.victim_steamid = Some(format!("{}", u64::from(id)));
    }
}

/// Chat message
/// Matches:
///    0: Player
///    1: Message
pub const REGEX_CHAT: &str = r"^(?:\*DEAD\*)?(?:\(TEAM\))?\s?(.*)\s:\s\s(.*)$";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ChatMessage {
    pub player_name: String,
    pub steamid: Option<String>,
    pub message: String,
}

impl ChatMessage {
    #[must_use]
    pub fn parse(caps: &Captures) -> Self {
        Self {
            player_name: caps[1].into(),
            steamid: None,
            message: caps[2].into(),
        }
    }

    pub fn set_steam_id(&mut self, id: SteamID) {
        self.steamid = Some(format!("{}", u64::from(id)));
    }
}

// Reads lines from output of the "status" command
// Includes players on server, player name, state, steamid, time connected
// If no player exists on the server with a steamid from here, it creates a new
// player and adds it to the list
pub const REGEX_STATUS: &str =
    r#"^#\s*(\d+)\s"(.*)"\s+(\[U:\d:\d+\])\s+((?:[\ds]+:?)+)\s+(\d+)\s*(\d+)\s*(\w+).*$"#;

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct StatusLine {
    pub userid: String,
    pub name: String,
    pub steamid: SteamID,
    pub time: u32,
    pub ping: u32,
    pub loss: u32,
    pub state: PlayerState,
}

impl StatusLine {
    /// # Errors
    /// If it contains an invalid `SteamID`
    pub fn parse(caps: &Captures) -> Result<Self> {
        let player_state = if caps[7].eq("spawning") {
            PlayerState::Spawning
        } else {
            PlayerState::Active
        };

        Ok(Self {
            userid: caps[1].into(),
            name: caps[2].into(),
            steamid: SteamID::from_steam3(&caps[3]).context("Failed to decode steamid.")?,
            time: get_time(&caps[4]).unwrap_or(0),
            ping: caps[5].parse().unwrap_or(0),
            loss: caps[6].parse().unwrap_or(0),
            state: player_state,
        })
    }
}

// Reads lines printed when demo recording terminates
// Example: Completed demo, recording time 1.8, game frames 115.
pub const REGEX_DEMOSTOP: &str = r"Completed demo, recording time ([\d.]+), game frames (\d+).";

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DemoStop {
    pub seconds: f32,
    pub frames: u32,
}

impl DemoStop {
    #[must_use]
    pub fn parse(caps: &Captures) -> Self {
        Self {
            seconds: caps[1].parse().unwrap_or(0.0),
            frames: caps[2].parse().unwrap_or(0),
        }
    }
}

// Converts a given string time (e.g. 57:48 or 1:14:46) as an integer number of
// seconds
#[allow(clippy::cast_possible_truncation)]
fn get_time(input: &str) -> Option<u32> {
    let mut t: u32 = 0;

    let splits: Vec<&str> = input.split(':').collect();
    let n = splits.len();

    for (i, v) in splits.iter().enumerate() {
        let dt = v.parse::<u32>().ok()?;
        t += 60u32.pow((n - i - 1) as u32) * dt;
    }

    Some(t)
}

#[allow(dead_code)]
const INVIS_CHARS: &[char] = &[
    '\u{00a0}',
    '\u{00ad}',
    '\u{034f}',
    '\u{061c}',
    '\u{115f}',
    '\u{1160}',
    '\u{17b4}',
    '\u{17b5}',
    '\u{180e}',
    '\u{2000}',
    '\u{2001}',
    '\u{2002}',
    '\u{2003}',
    '\u{2004}',
    '\u{2005}',
    '\u{2006}',
    '\u{2007}',
    '\u{2008}',
    '\u{2009}',
    '\u{200a}',
    '\u{200b}',
    '\u{200c}',
    '\u{200d}',
    '\u{200e}',
    '\u{200f}',
    '\u{202f}',
    '\u{205f}',
    '\u{2060}',
    '\u{2061}',
    '\u{2062}',
    '\u{2063}',
    '\u{2064}',
    '\u{206a}',
    '\u{206b}',
    '\u{206c}',
    '\u{206d}',
    '\u{206e}',
    '\u{206f}',
    '\u{3000}',
    '\u{2800}',
    '\u{3164}',
    '\u{feff}',
    '\u{ffa0}',
    '\u{1d159}',
    '\u{1d173}',
    '\u{1d174}',
    '\u{1d175}',
    '\u{1d176}',
    '\u{1d177}',
    '\u{1d178}',
    '\u{1d179}',
    '\u{1d17a}',
];
