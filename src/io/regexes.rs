#![allow(non_upper_case_globals)]
#![allow(unused_variables)]

use std::sync::Arc;

use regex::Captures;
use steamid_ng::{SteamID, SteamIDError};

use crate::player::{PlayerState, Team};

/*
    Useful commands:
        status
        tf_lobby_debug
        tf_party_debug //Not sure if this is actually useful, not really necessary

        callvote kick <userid>
        vote option<1/2> // Can't really use

*/

pub const REGEX_HOSTNAME: &str = r#"^hostname: (.*)$"#;
#[derive(Debug)]
pub struct Hostname(pub Arc<str>);
impl Hostname {
    pub fn parse(caps: Captures) -> Hostname {
        Hostname(caps[1].into())
    }
}

pub const REGEX_IP: &str = r#"^udp/ip  : (.*)$"#;
#[derive(Debug)]
pub struct ServerIP(pub Arc<str>);
impl ServerIP {
    pub fn parse(caps: Captures) -> ServerIP {
        ServerIP(caps[1].into())
    }
}

pub const REGEX_MAP: &str = r#"^map     : (.+) at: .*$"#;
#[derive(Debug)]
pub struct Map(pub Arc<str>);
impl Map {
    pub fn parse(caps: Captures) -> Map {
        Map(caps[1].into())
    }
}

pub const REGEX_PLAYERCOUNT: &str = r#"^players : (\d+) humans, (\d+) bots \((\d+) max\)$"#;
#[derive(Debug)]
pub struct PlayerCount {
    pub players: u32,
    pub bots: u32,
    pub max: u32,
}

impl PlayerCount {
    pub fn parse(caps: Captures) -> PlayerCount {
        PlayerCount {
            players: caps[1].parse().unwrap(),
            bots: caps[2].parse().unwrap(),
            max: caps[3].parse().unwrap(),
        }
    }
}

/// Player killed someone
/// Matches:
///    0: Killer
///    1: Victim
///    2: Weapon
///    3: Crit?
pub const REGEX_KILL: &str = r#"^(.*)\skilled\s(.*)\swith\s(.*)\.(\s\(crit\))?$"#;
#[derive(Debug)]
pub struct PlayerKill {
    pub killer_name: Arc<str>,
    pub killer_steamid: Option<Arc<str>>,
    pub victim_name: Arc<str>,
    pub victim_steamid: Option<Arc<str>>,
    pub weapon: Arc<str>,
    pub crit: bool,
}

impl PlayerKill {
    pub fn parse(caps: Captures) -> PlayerKill {
        PlayerKill {
            killer_name: caps[1].into(),
            killer_steamid: None,
            victim_name: caps[2].into(),
            victim_steamid: None,
            weapon: caps[3].into(),
            crit: caps.get(4).is_some(),
        }
    }
}

/// Chat message
/// Matches:
///    0: Player
///    1: Message
pub const REGEX_CHAT: &str = r#"^(?:\*DEAD\*)?(?:\(TEAM\))?\s?(.*)\s:\s\s(.*)$"#;

#[derive(Debug)]
pub struct ChatMessage {
    pub player_name: String,
    pub steamid: Option<String>,
    pub message: String,
}

impl ChatMessage {
    pub fn parse(caps: Captures) -> ChatMessage {
        ChatMessage {
            player_name: caps[1].to_string(),
            steamid: None,
            message: caps[2].to_string(),
        }
    }
}

// Reads lines from output of the "status" command
// Includes players on server, player name, state, steamid, time connected
// If no player exists on the server with a steamid from here, it creates a new player and adds it to the list
pub const REGEX_STATUS: &str =
    r#"^#\s*(\d+)\s"(.*)"\s+(\[U:\d:\d+\])\s+(\d*:?\d\d:\d\d)\s+(\d+)\s*(\d+)\s*(\w+).*$"#;

#[derive(Debug)]
pub struct StatusLine {
    pub userid: Arc<str>,
    pub name: Arc<str>,
    pub steamid: SteamID,
    pub time: u32,
    pub ping: u32,
    pub loss: u32,
    pub state: PlayerState,
}

impl StatusLine {
    pub fn parse(caps: Captures) -> Result<StatusLine, SteamIDError> {
        let mut player_state = PlayerState::Spawning;
        if caps[7].eq("active") {
            player_state = PlayerState::Active;
        }

        Ok(StatusLine {
            userid: caps[1].into(),
            name: caps[2].into(),
            steamid: SteamID::from_steam3(&caps[3])?,
            time: get_time(&caps[4]).unwrap_or(0),
            ping: caps[5].parse().unwrap(),
            loss: caps[6].parse().unwrap(),
            state: player_state,
        })
    }
}

// Converts a given string time (e.g. 57:48 or 1:14:46) as an integer number of seconds
fn get_time(input: &str) -> Option<u32> {
    let mut t: u32 = 0;

    let splits: Vec<&str> = input.split(':').collect();
    let n = splits.len();

    for (i, v) in splits.iter().enumerate() {
        // let dt: u32 = v.parse::<u32>().expect(&format!("Had trouble parsing {} as u32", v));
        let dt = v.parse::<u32>();

        if dt.is_err() {
            return None;
        }

        t += 60u32.pow((n - i - 1) as u32) * dt.unwrap();
    }

    Some(t)
}

// Reads lines from output of the "tf_lobby_debug" command
// Includes the team of players on the server
// NOTE: Teams are stored as INVADERS/DEFENDERS and does not swap when Red/Blu swaps so it cannot
// be used to reliably check which team the user is on, it can only check relative to the user (same/opposite team)
pub const REGEX_LOBBY: &str =
    r#"^  Member\[(\d+)] (\[U:\d:\d+])  team = TF_GC_TEAM_(\w+)  type = MATCH_PLAYER\s*$"#;

#[derive(Debug)]
pub struct LobbyLine {
    pub steamid: SteamID,
    pub team: Team,
}

impl LobbyLine {
    pub fn parse(caps: &Captures) -> Result<LobbyLine, SteamIDError> {
        let mut team = Team::Unassigned;
        match &caps[3] {
            // TODO - This is not right since teams swap in maps like payload.
            // This is only temporary until the g15 command is implemented
            "INVADERS" => team = Team::Blu,
            "DEFENDERS" => team = Team::Red,
            _ => {}
        }

        Ok(LobbyLine {
            steamid: SteamID::from_steam3(&caps[2])?,
            team,
        })
    }
}

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
