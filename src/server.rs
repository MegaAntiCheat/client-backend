use serde::Serialize;
use std::sync::Arc;

use crate::{
    events::console::ConsoleOutput,
    io::regexes::{self, ChatMessage, PlayerKill},
};

// Server

pub struct Server {
    map: Option<Arc<str>>,
    ip: Option<Arc<str>>,
    hostname: Option<Arc<str>>,
    max_players: Option<u32>,
    num_players: Option<u32>,
    gamemode: Option<Gamemode>,
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
    pub fn new() -> Server {
        Server {
            map: None,
            ip: None,
            hostname: None,
            max_players: None,
            num_players: None,

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

    pub fn gamemode(&self) -> Option<&Gamemode> {
        self.gamemode.as_ref()
    }
}

impl Server {
    // **** Message handling ****

    /// Handles any io output from running commands / reading the console log file.
    /// Returns:
    /// * Some<[SteamID]> of a player if they have been newly added to the server.
    pub fn handle_console_output(&mut self, response: ConsoleOutput) {
        use ConsoleOutput::*;
        match response {
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
            G15(_) | Status(_) => {}
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
