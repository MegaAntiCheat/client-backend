use std::sync::Arc;

use serde::Serialize;

use crate::{
    console::ConsoleOutput,
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
    #[must_use]
    pub const fn new() -> Self {
        Self {
            map: None,
            ip: None,
            hostname: None,
            max_players: None,
            num_players: None,

            gamemode: None,
        }
    }

    // **** Getters / Setters ****

    #[must_use]
    pub fn map(&self) -> Option<Arc<str>> {
        self.map.clone()
    }

    #[must_use]
    pub fn ip(&self) -> Option<Arc<str>> {
        self.ip.clone()
    }

    #[must_use]
    pub fn hostname(&self) -> Option<Arc<str>> {
        self.hostname.clone()
    }

    #[must_use]
    pub const fn max_players(&self) -> Option<u32> {
        self.max_players
    }

    #[must_use]
    pub const fn num_players(&self) -> Option<u32> {
        self.num_players
    }

    #[must_use]
    pub const fn gamemode(&self) -> Option<&Gamemode> {
        self.gamemode.as_ref()
    }
}

impl Default for Server {
    fn default() -> Self {
        Self::new()
    }
}

impl Server {
    // **** Message handling ****

    /// Handles any io output from running commands / reading the console log
    /// file. Returns:
    /// * Some<`SteamID`> of a player if they have been newly added to the
    ///   server.
    pub fn handle_console_output(&mut self, response: ConsoleOutput) {
        use ConsoleOutput::{Chat, Hostname, Kill, Map, PlayerCount, ServerIP, Status, G15, DemoStop};
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
            G15(_) | Status(_) | DemoStop(_) => {}
        }
    }

    #[allow(clippy::unused_self)]
    #[allow(clippy::needless_pass_by_value)]
    fn handle_chat(&mut self, chat: ChatMessage) {
        // TODO
        tracing::debug!("Chat: {:?}", chat);
    }

    #[allow(clippy::unused_self)]
    #[allow(clippy::needless_pass_by_value)]
    fn handle_kill(&mut self, kill: PlayerKill) {
        // TODO
        tracing::debug!("Kill: {:?}", kill);
    }
}
