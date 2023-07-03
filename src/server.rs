use std::{collections::HashMap, sync::Arc};
use steamid_ng::SteamID;

use crate::player::Player;

#[derive(Debug)]
pub struct Server {
    pub hostname: Option<Arc<str>>,
    pub map: Option<Arc<str>>,
    pub ip: Option<Arc<str>>,
    pub max_players: Option<u32>,
    pub num_players: Option<u32>,
    pub players: HashMap<SteamID, Player>,
}

impl Server {
    pub fn new() -> Server {
        Server {
            hostname: None,
            map: None,
            ip: None,
            max_players: None,
            num_players: None,
            players: HashMap::new(),
        }
    }
}
