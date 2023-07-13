use std::fmt::Display;

use rcon::Connection;
use tokio::net::TcpStream;

use crate::state::State;

use super::Commands;

#[derive(Debug)]
pub enum KickReason {
    None,
    Idle,
    Cheating,
    Scamming,
}

impl Display for KickReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            KickReason::None => "other",
            KickReason::Idle => "idle",
            KickReason::Cheating => "cheating",
            KickReason::Scamming => "scamming",
        })
    }
}

pub struct CommandManager {
    rcon: Option<Connection<TcpStream>>,
}

impl CommandManager {
    pub fn new() -> CommandManager {
        CommandManager { rcon: None }
    }

    pub fn is_connected(&self) -> bool {
        self.rcon.is_some()
    }

    pub async fn try_connect(&mut self) -> Result<(), rcon::Error> {
        let password = State::read_state().settings.get_rcon_password();

        log::debug!("Attempting to reconnect to RCon");
        match Connection::connect("127.0.0.1:27015", &password).await {
            Ok(con) => {
                self.rcon = Some(con);
                log::debug!("RCon successfully connected");
                Ok(())
            }
            Err(e) => {
                self.rcon = None;
                log::debug!("RCon failed to connect: {:?}", e);
                Err(e)
            }
        }
    }

    pub async fn run_command(&mut self, command: &str) -> rcon::Result<String> {
        if self.rcon.is_none() {
            self.try_connect().await?;
        }

        log::debug!("Running command \"{}\"", command);
        let out = self.rcon.as_mut().unwrap().cmd(command).await;

        if let Err(e) = &out {
            self.rcon = None;
            log::debug!("Rcon connection was lost: {}", e);
        }

        out
    }

    pub fn send_chat_command(message: &str) -> String {
        format!("say \"{}\"", message)
    }
}
