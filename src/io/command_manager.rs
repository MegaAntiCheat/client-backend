use std::{fmt::Display, time::Duration};

use anyhow::{Context, Result};
use rcon::Connection;
use serde::Deserialize;
use tokio::{net::TcpStream, time::timeout};

use crate::state::State;

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KickReason {
    None,
    Idle,
    Cheating,
    Scamming,
}

impl Default for KickReason {
    fn default() -> Self {
        Self::None
    }
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

#[allow(dead_code)]
impl CommandManager {
    pub fn new() -> CommandManager {
        CommandManager { rcon: None }
    }

    pub fn is_connected(&self) -> bool {
        self.rcon.is_some()
    }

    pub async fn run_command(&mut self, command: &str) -> Result<String> {
        let rcon = if let Some(rcon) = self.rcon.as_mut() {
            rcon
        } else {
            self.try_connect()
                .await
                .context("Failed to reconnect to RCon.")?
        };

        tracing::debug!("Running command \"{}\"", command);
        rcon.cmd(command)
            .await
            .map_err(|e| {
                self.rcon = None;
                e
            })
            .context("Failed to run command")
    }

    async fn try_connect(&mut self) -> Result<&mut Connection<TcpStream>> {
        let password = State::read_state().settings.get_rcon_password();

        tracing::debug!("Attempting to reconnect to RCon");
        match timeout(
            Duration::from_secs(2),
            Connection::connect("127.0.0.1:27015", &password),
        )
        .await
        {
            Ok(Ok(con)) => {
                self.rcon = Some(con);
                tracing::info!("RCon reconnected.");
                Ok(self.rcon.as_mut().expect(""))
            }
            Ok(Err(e)) => {
                self.rcon = None;
                Err(e).context("Failed to establish connection")
            }
            Err(e) => {
                self.rcon = None;
                Err(e).context("RCon connection timed out")
            }
        }
    }
}
