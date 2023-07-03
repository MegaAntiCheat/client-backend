use std::{fmt::Display, sync::Arc};

use rcon::Connection;
use tokio::{net::TcpStream, runtime::Runtime};

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
    runtime: Runtime,
    rcon: Option<Connection<TcpStream>>,
    password: Arc<str>,
}

pub const CMD_STATUS: &str = "status";
pub const CMD_TF_LOBBY_DEBUG: &str = "tf_lobby_debug";

impl CommandManager {
    pub fn new(password: Arc<str>) -> CommandManager {
        let runtime = Runtime::new().unwrap();
        CommandManager {
            runtime,
            rcon: None,
            password,
        }
    }

    pub fn set_password(&mut self, password: Arc<str>) -> rcon::Result<()> {
        self.password = password;
        self.try_connect()
    }

    pub fn is_connected(&self) -> bool {
        self.rcon.is_some()
    }

    pub fn try_connect(&mut self) -> Result<(), rcon::Error> {
        log::debug!("Attempting to reconnect to RCon");
        let mut connected = Ok(());
        self.runtime.block_on(async {
            match Connection::connect("127.0.0.1:27015", &self.password).await {
                Ok(con) => {
                    self.rcon = Some(con);
                }
                Err(e) => {
                    self.rcon = None;
                    connected = Err(e);
                }
            }
        });

        match &connected {
            Ok(_) => log::debug!("RCon successfully connected"),
            Err(e) => log::debug!("RCon failed to connect: {:?}", e),
        }

        connected
    }

    pub fn run_command(&mut self, command: &str) -> rcon::Result<String> {
        let mut out = None;
        if self.rcon.is_none() {
            self.try_connect()?;
        }

        log::debug!("Running command \"{}\"", command);
        self.runtime.block_on(async {
            if let Some(rcon) = &mut self.rcon {
                out = Some(rcon.cmd(command).await);
            }
        });

        if out.as_ref().unwrap().is_err() {
            self.rcon = None;
        }

        out.unwrap()
    }

    pub fn kick_player_command(player_userid: &str, reason: KickReason) -> String {
        format!("callvote kick \"{} {}\"", player_userid, reason)
    }

    pub fn send_chat_command(message: &str) -> String {
        format!("say \"{}\"", message)
    }
}
