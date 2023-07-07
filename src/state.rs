use std::sync::RwLock;

use lazy_static::lazy_static;
use serde::{ser::SerializeMap, Serialize};

use crate::{io::IOResponse, server::Server};

lazy_static! {
    pub static ref STATE: RwLock<State> = RwLock::new(State::new());
}

pub struct State {
    pub log_file_state: std::io::Result<()>,
    pub rcon_state: Result<(), rcon::Error>,
    pub server: Server,
}

impl State {
    pub fn new() -> State {
        State {
            log_file_state: Ok(()),
            rcon_state: Ok(()),
            server: Server::new(),
        }
    }

    /// Handle a message from the IO thread
    pub fn handle_io_response(&mut self, response: IOResponse) {
        use IOResponse::*;
        match response {
            NoLogFile(e) => self.log_file_state = Err(e),
            LogFileOpened => self.log_file_state = Ok(()),
            NoRCON(e) => self.rcon_state = Err(e),
            RCONConnected => self.rcon_state = Ok(()),
            Lobby(_) | Status(_) | Chat(_) | Kill(_) | Hostname(_) | ServerIP(_) | Map(_)
            | PlayerCount(_) => {
                self.server.handle_io_response(response);
            }
        }
    }
}

impl Serialize for State {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let mut smap = serializer.serialize_map(Some(1))?;
        smap.serialize_entry("server", &self.server)?;
        smap.end()
    }
}
