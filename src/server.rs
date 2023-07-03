use serde::{ser::SerializeMap, Serialize};
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

impl Serialize for Server {
    fn serialize<S>(&self, s: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut smap = s.serialize_map(None)?;

        smap.serialize_entry("hostname", &self.hostname)?;
        smap.serialize_entry("map", &self.map)?;
        smap.serialize_entry("ip", &self.ip)?;
        smap.serialize_entry("maxPlayers", &self.max_players)?;
        smap.serialize_entry("numPlayers", &self.num_players)?;
        smap.serialize_entry("players", &self.players)?;

        smap.end()
    }
}
