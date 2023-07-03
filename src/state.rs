use std::sync::RwLock;

use lazy_static::lazy_static;

use crate::{
    io::{
        regexes::{self, ChatMessage, Hostname, PlayerKill, StatusLine},
        IOResponse,
    },
    server::Server,
};

lazy_static! {
    pub static ref STATE: RwLock<State> = RwLock::new(State::new());
}

pub struct State {
    log_file_state: std::io::Result<()>,
    rcon_state: Result<(), rcon::Error>,
    server: Server,
}

impl State {
    pub fn new() -> State {
        State {
            log_file_state: Ok(()),
            rcon_state: Ok(()),
            server: Server::new(),
        }
    }

    pub fn handle_io_response(&mut self, response: IOResponse) {
        match response {
            IOResponse::NoLogFile(e) => self.log_file_state = Err(e),
            IOResponse::LogFileOpened => self.log_file_state = Ok(()),
            IOResponse::NoRCON(e) => self.rcon_state = Err(e),
            IOResponse::RCONConnected => self.rcon_state = Ok(()),
            IOResponse::Status(status) => self.handle_status_line(status),
            IOResponse::Chat(chat) => self.handle_chat(chat),
            IOResponse::Kill(kill) => self.handle_kill(kill),
            IOResponse::Hostname(regexes::Hostname(hostname)) => {
                self.server.hostname = Some(hostname);
            }
            IOResponse::ServerIP(regexes::ServerIP(ip)) => {
                self.server.ip = Some(ip);
            }
            IOResponse::Map(regexes::Map(map)) => {
                self.server.map = Some(map);
            }
            IOResponse::PlayerCount(playercount) => {
                self.server.max_players = Some(playercount.max);
                self.server.num_players = Some(playercount.players);
            }
        }
    }

    fn handle_status_line(&mut self, status: StatusLine) {
        // TODO
        log::info!("Status: {:?}", status);
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
