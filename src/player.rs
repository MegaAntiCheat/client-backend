use serde::{ser::SerializeMap, Serialize};
use std::sync::Arc;
use steamid_ng::SteamID;

#[derive(Debug, Clone, Copy, Eq, PartialEq, Serialize)]
pub enum PlayerState {
    Active,
    Spawning,
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub enum Team {
    Unassigned = 0,
    Spectators = 1,
    RED = 2,
    BLU = 3,
}

impl Serialize for Team {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        s.serialize_u32(*self as u32)
    }
}

#[derive(Debug)]
pub struct SteamInfo {
    account_name: Arc<str>,
    pfp: Arc<str>,
    // TODO
}

#[derive(Debug, Clone, Serialize)]
pub struct GameInfo {
    pub userid: Arc<str>,
    pub team: Team,
    pub ping: u32,
    pub loss: u32,
    pub state: PlayerState,
    pub kills: u32,
    pub deaths: u32,
}

#[derive(Debug)]
pub struct Player {
    pub steamid: SteamID,
    pub name: Arc<str>,
    pub game_info: GameInfo,
}

impl Serialize for Player {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut smap = s.serialize_map(None)?;

        smap.serialize_entry("name", self.name.as_ref())?;
        smap.serialize_entry("gameInfo", &self.game_info)?;

        smap.end()
    }
}
