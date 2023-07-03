use std::sync::Arc;
use steamid_ng::SteamID;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum PlayerState {
    Active,
    Spawning,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Team {
    Unassigned = 0,
    Spectators = 1,
    RED = 2,
    BLU = 3,
}

#[derive(Debug)]
pub struct SteamInfo {
    account_name: Arc<str>,
    pfp: Arc<str>,
    // TODO
}

#[derive(Debug)]
pub struct Player {
    steamid: SteamID,
    name: Arc<str>,
    team: Team,
}
