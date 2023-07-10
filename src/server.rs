use serde::{Serialize, Serializer};
use std::{
    collections::{HashMap, VecDeque},
    ops::Range,
    sync::Arc,
};
use steamid_ng::SteamID;

use crate::{
    io::{
        regexes::{self, ChatMessage, LobbyLine, PlayerKill, StatusLine},
        IOOutput,
    },
    player::{Player, SteamInfo},
};

const MAX_HISTORY_LEN: usize = 100;

// Server

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    map: Option<Arc<str>>,
    ip: Option<Arc<str>>,
    hostname: Option<Arc<str>>,
    max_players: Option<u32>,
    num_players: Option<u32>,
    #[serde(serialize_with = "serialize_player_map")]
    players: HashMap<SteamID, Player>,
    #[serde(skip)]
    player_history: VecDeque<Player>,
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
            player_history: VecDeque::with_capacity(MAX_HISTORY_LEN),
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

    /// Moves any old players from the server into history. Any console commands (status, tf_lobby_debug, etc)
    /// should be run before calling this function again to prevent removing all players from the player list.
    pub fn refresh(&mut self) {
        // Get old players
        let unaccounted_players: Vec<Player> = self
            .players
            .values()
            .filter(|p| !p.game_info.accounted)
            .cloned()
            .collect();

        // Remove old players from server
        for p in &unaccounted_players {
            self.players.remove(&p.steamid);
        }

        // Remove any of them from the history as they will be added more recently
        self.player_history
            .retain(|p| !unaccounted_players.iter().any(|up| up.steamid == p.steamid));

        // Shrink to not go past max number of players
        let num_players = self.player_history.len() + unaccounted_players.len();
        for _ in MAX_HISTORY_LEN..num_players {
            self.player_history.pop_front();
        }

        for p in unaccounted_players {
            self.player_history.push_back(p);
        }

        // Mark all remaining players as unaccounted, they will be marked as accounted again
        // when they show up in status or another console command.
        for p in self.players.values_mut() {
            p.game_info.accounted = false;
        }
    }

    pub fn insert_steam_info(&mut self, player: SteamID, info: SteamInfo) {
        if let Some(player) = self.players.get_mut(&player) {
            player.steam_info = Some(info);
        }
    }

    pub fn get_history(&self, range: Range<usize>) -> Vec<&Player> {
        self.player_history
            .iter()
            .rev()
            .skip(range.start)
            .take(range.end - range.start)
            .collect()
    }

    fn handle_lobby_line(&mut self, lobby: LobbyLine) {
        if let Some(player) = self.players.get_mut(&lobby.steamid) {
            player.game_info.team = lobby.team;
            player.game_info.accounted = true;
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
            player.game_info.accounted = true;
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
