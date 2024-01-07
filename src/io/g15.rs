#![allow(non_upper_case_globals)]
#![allow(unused_variables)]

use std::sync::Arc;

use anyhow::Result;
use regex::{Captures, Regex};
use steamid_ng::SteamID;
use thiserror::Error;

use crate::player::Team;

#[derive(Debug, Error, Clone)]
pub enum G15Error {
    /// Occurs when m_someArray\[X\] has some X greater than 33 (since these are static 34 element arrays)
    #[error("index provided from output of g15 command was invalid")]
    IndexOutOfBounds,
}

/// July 25, 2023 TF2 update updated the maximum supported number of players in a single server instance to 100,
/// from 32. This is unlikely to ever change again, so rather than dynamically handling g15_dumpplayer logs that
/// can contain any number of array entries, we simply hard-code the current maximum amount of players you will
/// EVER find in a game.
const MAX_POSSIBLE_PLAYERS: u8 = 100;

/// A RegMatch struct contains a Regex and the corresponding function to add the result to the G15Player vec
/// if the regex results in a match.
pub(crate) struct RegMatch(
    Regex,
    fn(caps: Captures, players: &mut [G15Player]) -> Result<()>,
);
impl RegMatch {
    pub fn new(
        reg_str: &str,
        func: fn(caps: Captures, players: &mut [G15Player]) -> Result<()>,
    ) -> RegMatch {
        // Unwrap, because these Regex's should statically compile and we want to panic if they don't.
        let regers = Regex::new(reg_str).expect("Regex compilation");
        RegMatch(regers, func)
    }
}

/// `m_iAmmo[1] integer (32)` --> capture groups: `(player idx)` `(ammo count)`
pub(crate) const REGEX_I_AMMO: &str = r#"^m_iAmmo\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub(crate) fn parse_ammo(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let ammo: u32 = caps[2].parse()?;
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.ammo = Some(ammo);
    Ok(())
}

/// `m_szName[1] string (Lust)` --> capture groups: `(player idx)` `(string name)`
pub(crate) const REGEX_SZ_NAME: &str = r#"^m_szName\[(\d+)\]\s+string\s+\((.+)\)$"#;
pub(crate) fn parse_name(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let name = caps[2].into();
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.name = Some(name);
    Ok(())
}

/// `m_iPing[2] integer (15)` --> capture groups: `(player idx)` `(ping in ms)`
pub(crate) const REGEX_I_PING: &str = r#"^m_iPing\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub(crate) fn parse_ping(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let ping: u32 = caps[2].parse()?;
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.ping = Some(ping);
    Ok(())
}

/// `m_iScore[4] integer (7)` --> capture groups: `(player idx)` `(score (num of kills))`
pub(crate) const REGEX_I_SCORE: &str = r#"^m_iScore\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub(crate) fn parse_score(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let score: u32 = caps[2].parse()?;
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.score = Some(score);
    Ok(())
}

/// `m_iDeaths[17] integer (4)` --> capture groups: `(player idx)` `(deaths)`
pub(crate) const REGEX_I_DEATHS: &str = r#"^m_iDeaths\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub(crate) fn parse_deaths(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let deaths: u32 = caps[2].parse()?;
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.deaths = Some(deaths);
    Ok(())
}

/// `m_bConnected[0] bool (false)` --> capture groups:  `(player idx)` `(connected status (true/false))`
pub(crate) const REGEX_B_CONNECTED: &str = r#"^m_bConnected\[(\d+)\]\s+bool\s+\((false|true)\)$"#;
pub(crate) fn parse_connected(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let connected_str = &caps[2];
    let truth_value = matches!(connected_str, "true");
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.connected = Some(truth_value);
    Ok(())
}

/// `m_iTeam[18] integer (3)` --> capture groups: `(player idx)` `(team (0=unconnected, 1=spectator, 2=blu, 3=red))`
pub(crate) const REGEX_I_TEAM: &str = r#"^m_iTeam\[(\d+)\]\s+integer\s+\(([0-3])\)$"#;
pub(crate) fn parse_team(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let team: u32 = caps[2].parse()?;
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.team = Team::try_from(team).ok();
    Ok(())
}

/// `m_bAlive[0] bool (false)` --> capture groups: `(player idx)` `(alive status (true/false))`
pub(crate) const REGEX_B_ALIVE: &str = r#"^m_bAlive\[(\d+)\]\s+bool\s+\((false|true)\)$"#;
pub(crate) fn parse_alive(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let alive_str = &caps[2];
    let truth_value = matches!(alive_str, "true");
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.alive = Some(truth_value);
    Ok(())
}

/// `m_iHealth[3] integer (125)` --> capture groups: `(player idx)` `(current health)`
pub(crate) const REGEX_I_HEALTH: &str = r#"^m_iHealth\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub(crate) fn parse_health(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let health: u32 = caps[2].parse()?;
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.health = Some(health);
    Ok(())
}

/// `m_iAccountID[3] integer (1505713148)` --> capture groups: `(player idx)` `(variable component of a steamID3 ([U:1:1505713148]))`
pub(crate) const REGEX_I_SID3: &str = r#"^m_iAccountID\[(\d+)\]\s+integer\s+\((\d{4,})\)$"#;
pub(crate) fn parse_sid3(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let sid3: u64 = caps[2].parse()?;
    let steamid = SteamID::from_steam3(&format!("[U:1:{}]", sid3))?;
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.steamid = Some(steamid);
    Ok(())
}

/// `m_bValid[24] bool (true)` --> capture groups: `(player idx)` `(valid status (true/false))`
/// I'm not sure what valid means - perhaps to with whether this is a current/in-game player idx
/// and to display this row in the scoreboard or not? Can someone look pls
pub(crate) const REGEX_B_VALID: &str = r#"^m_bValid\[(\d+)\]\s+bool\s+\((false|true)\)$"#;
pub(crate) fn parse_valid(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let valid_str: &str = &caps[2];
    let truth_value: bool = matches!(valid_str, "true");
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.valid = Some(truth_value);
    Ok(())
}

/// `m_iUserID[2] integer (68)` --> capture groups: `(player idx)` `(server id (as it appears in status))`
pub(crate) const REGEX_I_USERID: &str = r#"^m_iUserID\[(\d+)\]\s+integer\s+\((\d+)\)$"#;
pub(crate) fn parse_userid(caps: Captures, players: &mut [G15Player]) -> Result<()> {
    let idx: usize = caps[1].parse()?;
    let userid = caps[2].into();
    let player_ref = players.get_mut(idx).ok_or(G15Error::IndexOutOfBounds)?;
    player_ref.userid = Some(userid);
    Ok(())
}

/// The G15Player struct contains all the data per player searched.
/// Each of the elements may be None, as the parsing could fail on a particular line, leaving the value undefined.
/// We still want the rest of the useful data though
#[derive(Debug, Clone)]
pub struct G15Player {
    pub name: Option<Arc<str>>,   // eg "Lilith"
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
    pub userid: Option<Arc<str>>, // eg "301"
}
impl G15Player {
    fn new() -> G15Player {
        G15Player {
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
}
impl Default for G15Player {
    fn default() -> Self {
        Self::new()
    }
}

/// The G15Parser struct contains a vector of compiled Regex automata to parse strings out of
/// the g15_dumpplayer console output.
/// It implements a `parse_g15` fn to use these to parse useful player data from the output.
pub(crate) struct G15Parser {
    patterns: Vec<RegMatch>,
}
impl G15Parser {
    pub fn new() -> G15Parser {
        G15Parser {
            patterns: vec![
                RegMatch::new(REGEX_I_AMMO, parse_ammo),
                RegMatch::new(REGEX_SZ_NAME, parse_name),
                RegMatch::new(REGEX_I_PING, parse_ping),
                RegMatch::new(REGEX_I_SCORE, parse_score),
                RegMatch::new(REGEX_I_DEATHS, parse_deaths),
                RegMatch::new(REGEX_B_CONNECTED, parse_connected),
                RegMatch::new(REGEX_I_TEAM, parse_team),
                RegMatch::new(REGEX_B_ALIVE, parse_alive),
                RegMatch::new(REGEX_I_HEALTH, parse_health),
                RegMatch::new(REGEX_I_SID3, parse_sid3),
                RegMatch::new(REGEX_B_VALID, parse_valid),
                RegMatch::new(REGEX_I_USERID, parse_userid),
            ],
        }
    }

    /// Parse a g15_dumpplayer string via Regex search.
    /// We only extract useful data here, so drop most data.
    pub fn parse_g15(&self, g15_log: &str) -> Vec<G15Player> {
        let mut players: Vec<G15Player> =
            vec![G15Player::new(); (MAX_POSSIBLE_PLAYERS + 2) as usize];
        let lines = g15_log.split('\n');

        for line in lines {
            for pat in &self.patterns {
                // Perform a regex search on the current line using the stored regex in the current RegMatch struct
                let pat_caps = pat.0.captures(line.trim());
                if let Some(pat_caps) = pat_caps {
                    // Call the corresponding function to store the capture groups, as we have found a match
                    match pat.1(pat_caps, &mut players) {
                        Err(why) => tracing::error!("Parse error - {} at line {}", why, line),
                        Ok(value) => (),
                    }
                }
            }
        }
        // Only get players who are "valid" and have a non-zero userid
        players.retain(|x| x.valid.unwrap_or(false) && x.userid.as_deref().unwrap_or("0") != "0");

        players
    }
}

impl Default for G15Parser {
    fn default() -> Self {
        Self::new()
    }
}
