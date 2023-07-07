use serde::{Serialize, Serializer};
use std::{collections::HashMap, sync::Arc};
use steamid_ng::SteamID;

use crate::{
    io::{
        regexes::{self, ChatMessage, LobbyLine, PlayerKill, StatusLine},
        IOResponse,
    },
    player::{GameInfo, Player, Team},
};

// Server

#[derive(Debug, Serialize)]
pub struct Server {
    pub map: Option<Arc<str>>,
    pub ip: Option<Arc<str>>,
    pub hostname: Option<Arc<str>>,
    #[serde(rename = "maxPlayers")]
    pub max_players: Option<u32>,
    #[serde(rename = "numPlayers")]
    pub num_players: Option<u32>,
    #[serde(serialize_with = "serialize_player_map")]
    pub players: HashMap<SteamID, Player>,
    pub gamemode: Option<Gamemode>,
}

#[derive(Debug, Serialize)]
pub struct Gamemode {
    pub matchmaking: bool,
    #[serde(rename = "type")]
    pub game_type: Arc<str>,
    pub vanilla: bool,
}

impl Server {
    pub fn new() -> Server {
        Server {
            map: None,
            ip: None,
            hostname: None,
            max_players: None,
            num_players: None,
            players: HashMap::new(),
            gamemode: None,
        }
    }

    pub fn handle_io_response(&mut self, response: IOResponse) {
        use IOResponse::*;
        match response {
            Lobby(lobby) => self.handle_lobby_line(lobby),
            Status(status) => self.handle_status_line(status),
            Chat(chat) => self.handle_chat(chat),
            Kill(kill) => self.handle_kill(kill),
            Hostname(regexes::Hostname(hostname)) => {
                self.hostname = Some(hostname);
            }
            ServerIP(regexes::ServerIP(ip)) => {
                self.ip = Some(ip);
            }
            Map(regexes::Map(map)) => {
                self.map = Some(map);
            }
            PlayerCount(playercount) => {
                self.max_players = Some(playercount.max);
                self.num_players = Some(playercount.players);
            }
            _ => {
                log::error!("Server was given a response it's not responsible for.");
            }
        }
    }

    fn handle_lobby_line(&mut self, lobby: LobbyLine) {
        if let Some(player) = self.players.get_mut(&lobby.steamid) {
            player.game_info.team = lobby.team;
        }
    }

    fn handle_status_line(&mut self, status: StatusLine) {
        log::debug!("Status: {:?}", &status);

        // Update existing player or insert new player
        if let Some(player) = self.players.get_mut(&status.steamid) {
            player.name = status.name;
            player.game_info.userid = status.userid;
            player.game_info.ping = status.ping;
            player.game_info.loss = status.loss;
            player.game_info.state = status.state;
            player.game_info.time = status.time;
        } else {
            // Since we have already gotten a valid steamid from this status line it is safe to unwrap
            let player = Player::new(&status, None).unwrap();
            self.players.insert(status.steamid, player);
        }
    }

    fn handle_chat(&mut self, chat: ChatMessage) {
        // TODO
        log::info!("Chat: {:?}", chat);
    }

    fn handle_kill(&mut self, kill: PlayerKill) {
        // TODO
        log::info!("Kill: {:?}", kill);
    }
}

// Useful

fn serialize_player_map<S, K, V>(map: &HashMap<K, V>, s: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
    V: Serialize,
{
    let values: Vec<_> = map.values().collect();
    values.serialize(s)
}
