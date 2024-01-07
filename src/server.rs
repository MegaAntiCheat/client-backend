use serde::Serialize;
use std::sync::Arc;
use steamid_ng::SteamID;

use crate::{
    io::{
        g15,
        regexes::{self, ChatMessage, PlayerKill, StatusLine},
        IOOutput,
    },
    player::{GameInfo, Players},
    player_records::PlayerRecords,
};

// Server

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Server {
    map: Option<Arc<str>>,
    ip: Option<Arc<str>>,
    hostname: Option<Arc<str>>,
    max_players: Option<u32>,
    num_players: Option<u32>,
    gamemode: Option<Gamemode>,
    players: Players,
}

#[derive(Debug, Serialize, Clone)]
pub struct Gamemode {
    pub matchmaking: bool,
    #[serde(rename = "type")]
    pub game_type: Arc<str>,
    pub vanilla: bool,
}

#[allow(dead_code)]
impl Server {
    pub fn new(playerlist: PlayerRecords) -> Server {
        Server {
            map: None,
            ip: None,
            hostname: None,
            max_players: None,
            num_players: None,
            players: Players::new(playerlist),

            gamemode: None,
        }
    }

    // **** Getters / Setters ****

    pub fn map(&self) -> Option<Arc<str>> {
        self.map.clone()
    }

    pub fn ip(&self) -> Option<Arc<str>> {
        self.ip.clone()
    }

    pub fn hostname(&self) -> Option<Arc<str>> {
        self.hostname.clone()
    }

    pub fn max_players(&self) -> Option<u32> {
        self.max_players
    }

    pub fn num_players(&self) -> Option<u32> {
        self.num_players
    }

    pub fn players(&self) -> &Players {
        &self.players
    }

    pub fn players_mut(&mut self) -> &mut Players {
        &mut self.players
    }

    pub fn gamemode(&self) -> Option<&Gamemode> {
        self.gamemode.as_ref()
    }
}

impl Server {
    // **** Message handling ****

    /// Handles any io output from running commands / reading the console log file.
    /// Returns:
    /// * Some<[SteamID]> of a player if they have been newly added to the server.
    pub fn handle_io_output(&mut self, response: IOOutput) -> Vec<SteamID> {
        use IOOutput::*;
        match response {
            G15(players) => return self.handle_g15_parse(players).into(),
            Status(status) => {
                return self
                    .handle_status_line(status)
                    .map(|s| vec![s])
                    .unwrap_or_default();
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

        Vec::new()
    }

    fn handle_g15_parse(&mut self, players: Vec<g15::G15Player>) -> Vec<SteamID> {
        let mut new_players = Vec::new();
        for g15 in players {
            if g15.steamid.is_none() {
                continue;
            }
            let steamid = g15.steamid.unwrap();

            // Add to connected players if they aren't already
            if !self.players.connected.contains(&steamid) {
                self.players.connected.push(steamid);
            }

            // Update game info
            if let Some(game_info) = self.players.game_info.get_mut(&steamid) {
                if let Some(name) = g15.name.as_ref() {
                    if *name != game_info.name {
                        self.players.records.update_name(&steamid, name.clone());
                    }
                }
                game_info.update_from_g15(g15);
            } else if let Some(game_info) = GameInfo::new_from_g15(g15) {
                // Update name
                self.players
                    .records
                    .update_name(&steamid, game_info.name.clone());

                self.players.game_info.insert(steamid, game_info);
                new_players.push(steamid);
            }
        }

        new_players
    }

    /// Given a status line, update an existing or add a new one to the server.
    /// Returns the SteamID if a new player was created.
    fn handle_status_line(&mut self, status: StatusLine) -> Option<SteamID> {
        let steamid = status.steamid;

        // Add to connected players if they aren't already
        if !self.players.connected.contains(&steamid) {
            self.players.connected.push(steamid);
        }

        if let Some(game_info) = self.players.game_info.get_mut(&steamid) {
            if status.name != game_info.name {
                self.players
                    .records
                    .update_name(&steamid, status.name.clone());
            }

            game_info.update_from_status(status);
            None
        } else {
            let game_info = GameInfo::new_from_status(status);

            // Update name
            self.players
                .records
                .update_name(&steamid, game_info.name.clone());

            self.players.game_info.insert(steamid, game_info);
            Some(steamid)
        }
    }

    fn handle_chat(&mut self, chat: ChatMessage) {
        // TODO
        tracing::debug!("Chat: {:?}", chat);
    }

    fn handle_kill(&mut self, kill: PlayerKill) {
        // TODO
        tracing::debug!("Kill: {:?}", kill);
    }
}
