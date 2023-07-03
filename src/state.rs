use std::sync::RwLock;

use lazy_static::lazy_static;
use serde::{ser::SerializeMap, Serialize, __private::ser::serialize_tagged_newtype};
use steamid_ng::SteamID;

use crate::{
    io::{
        regexes::{self, ChatMessage, Hostname, PlayerKill, StatusLine},
        IOResponse,
    },
    player::{GameInfo, Player, Team},
    server::Server,
};

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
        log::debug!("Status: {:?}", &status);

        match SteamID::from_steam3(&status.steamid) {
            Ok(steamid) => {
                // Update existing player or insert new player
                if let Some(player) = self.server.players.get_mut(&steamid) {
                    player.game_info.userid = status.userid;
                    player.name = status.name;
                    player.game_info.ping = status.ping;
                    player.game_info.loss = status.loss;
                    player.game_info.state = status.state;
                } else {
                    let player = Player {
                        steamid,
                        name: status.name,
                        game_info: GameInfo {
                            userid: status.userid,
                            team: Team::Unassigned,
                            ping: status.ping,
                            loss: status.loss,
                            state: status.state,
                            kills: 0,
                            deaths: 0,
                        },
                    };

                    self.server.players.insert(steamid, player);
                }
            }
            Err(e) => {
                log::error!("Invalid SteamID {}: {}", status.steamid, e);
            }
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
