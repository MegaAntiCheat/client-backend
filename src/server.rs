use serde::{Serialize, Serializer};
use std::{collections::HashMap, sync::Arc};
use steamid_ng::SteamID;

use crate::{
    io::{
        regexes::{self, ChatMessage, LobbyLine, PlayerKill, StatusLine},
        IOOutput,
    },
    player::{Player, SteamInfo},
};

// Server

#[derive(Debug, Serialize)]
pub struct Server {
    map: Option<Arc<str>>,
    ip: Option<Arc<str>>,
    hostname: Option<Arc<str>>,
    #[serde(rename = "maxPlayers")]
    max_players: Option<u32>,
    #[serde(rename = "numPlayers")]
    num_players: Option<u32>,
    #[serde(serialize_with = "serialize_player_map")]
    players: HashMap<SteamID, Player>,
    gamemode: Option<Gamemode>,
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

    pub fn handle_io_response(&mut self, response: IOOutput) -> Option<SteamID> {
        // TODO - Maybe move this back into state instead of inside server?
        use IOOutput::*;
        match response {
            Lobby(lobby) => self.handle_lobby_line(lobby),
            Status(status) => {
                return self.add_or_update_player(status, None);
            }
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
        }

        None
    }

    pub fn insert_steam_info(&mut self, player: SteamID, info: SteamInfo) {
        if let Some(player) = self.players.get_mut(&player) {
            player.steam_info = Some(info);
        }
    }

    fn handle_lobby_line(&mut self, lobby: LobbyLine) {
        if let Some(player) = self.players.get_mut(&lobby.steamid) {
            player.game_info.team = lobby.team;
        }
    }

    /// Given a status line, update an existing or add a new one to the server.
    /// Returns the SteamID if a new player was created.
    fn add_or_update_player(
        &mut self,
        status: StatusLine,
        user: Option<&SteamID>,
    ) -> Option<SteamID> {
        // Update existing player or insert new player
        if let Some(player) = self.players.get_mut(&status.steamid) {
            player.name = status.name;
            player.game_info.userid = status.userid;
            player.game_info.ping = status.ping;
            player.game_info.loss = status.loss;
            player.game_info.state = status.state;
            player.game_info.time = status.time;
            None
        } else {
            // Since we have already gotten a valid steamid from this status line it is safe to unwrap
            let player = Player::new_from_status(&status, user).unwrap();
            self.players.insert(status.steamid, player);
            Some(status.steamid)
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
