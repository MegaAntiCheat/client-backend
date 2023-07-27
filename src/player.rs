use serde::Serialize;
use serde_json::Map;
use std::sync::Arc;
use steamid_ng::SteamID;

use crate::{
    io::regexes::StatusLine,
    player_records::{PlayerRecord, Verdict},
};

// Player

#[derive(Debug, Clone, Serialize)]
pub struct Player {
    pub name: Arc<str>,
    #[serde(rename = "steamID64")]
    pub steamid: SteamID,
    #[serde(rename = "isSelf")]
    pub is_self: bool,
    #[serde(rename = "gameInfo")]
    pub game_info: GameInfo,
    #[serde(rename = "steamInfo")]
    pub steam_info: Option<SteamInfo>,
    #[serde(rename = "customData")]
    pub custom_data: serde_json::Value,
    pub tags: Vec<Arc<str>>,
    #[serde(rename = "localVerdict")]
    pub local_verdict: Verdict,
    pub convicted: bool,
}

impl Player {
    pub fn new_from_status(status: &StatusLine, user: Option<SteamID>) -> Player {
        let is_self = user.map(|user| user == status.steamid).unwrap_or(false);
        Player {
            name: status.name.clone(),
            steamid: status.steamid,
            is_self,
            game_info: GameInfo::new(status),
            steam_info: None,
            custom_data: serde_json::Value::Object(Map::new()),
            tags: Vec::new(),
            local_verdict: Verdict::Player,
            convicted: false,
        }
    }

    /// Given a record, update this player with the included data.
    pub fn update_from_record(&mut self, record: PlayerRecord) {
        if record.steamid != self.steamid {
            tracing::error!("Updating player with wrong record.");
            return;
        }

        self.custom_data = record.custom_data;
        self.local_verdict = record.verdict;
    }

    /// Create a record from the current player
    #[allow(dead_code)]
    pub fn get_record(&self) -> PlayerRecord {
        PlayerRecord {
            steamid: self.steamid,
            custom_data: self.custom_data.clone(),
            verdict: self.local_verdict,
        }
    }
}

// PlayerState

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
pub enum PlayerState {
    Active,
    Spawning,
}

// Team

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Team {
    Unassigned = 0,
    Spectators = 1,
    Red = 2,
    Blu = 3,
}

impl TryFrom<u32> for Team {
    type Error = &'static str;
    fn try_from(val: u32) -> Result<Self, Self::Error> {
        match val {
            0 => Ok(Team::Unassigned),
            1 => Ok(Team::Spectators),
            2 => Ok(Team::Red),
            3 => Ok(Team::Blu),
            _ => Err("Not a valid team value"),
        }
    }
}

impl Serialize for Team {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        s.serialize_u32(*self as u32)
    }
}

// SteamInfo

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SteamInfo {
    #[serde(rename = "name")]
    pub account_name: Arc<str>,
    pub profile_url: Arc<str>,
    #[serde(rename = "pfp")]
    pub pfp_url: Arc<str>,
    pub pfp_hash: Arc<str>,
    pub profile_visibility: ProfileVisibility,
    pub time_created: Option<i64>,
    pub country_code: Option<Arc<str>>,

    pub vac_bans: i64,
    pub game_bans: i64,
    pub days_since_last_ban: Option<i64>,

    pub friends: Vec<Friend>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Friend {
    #[serde(rename = "steamID64")]
    pub steamid: SteamID,
    #[serde(rename = "friendSince")]
    pub friend_since: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum ProfileVisibility {
    Private = 1,
    FriendsOnly = 2,
    Public = 3,
}

impl From<i32> for ProfileVisibility {
    fn from(value: i32) -> Self {
        match value {
            1 => ProfileVisibility::Private,
            2 => ProfileVisibility::FriendsOnly,
            3 => ProfileVisibility::Public,
            _ => ProfileVisibility::Private,
        }
    }
}

// GameInfo

#[derive(Debug, Clone, Serialize)]
pub struct GameInfo {
    pub userid: Arc<str>,
    pub team: Team,
    pub time: u32,
    pub ping: u32,
    pub loss: u32,
    pub state: PlayerState,
    pub kills: u32,
    pub deaths: u32,

    #[serde(skip)]
    pub accounted: bool,
}

impl GameInfo {
    pub fn new(status: &StatusLine) -> GameInfo {
        GameInfo {
            userid: status.userid.clone(),
            team: Team::Unassigned,
            time: status.time,
            ping: status.ping,
            loss: status.loss,
            state: status.state,
            kills: 0,
            deaths: 0,

            accounted: true,
        }
    }
}
