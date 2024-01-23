use std::{fmt::Display, sync::Arc};

use event_loop::{try_get, HandlerStruct, Is, StateUpdater};
use rcon::Connection;
use serde::Deserialize;
use thiserror::Error;
use tokio::{net::TcpStream, sync::Mutex};

#[derive(Debug, Deserialize, Clone)]
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

#[derive(Debug, Error)]
pub enum CommandManagerError {
    #[error("RCon error {0}")]
    Rcon(#[from] rcon::Error),
    #[error("Rcon connection timeout: {0}")]
    TimeOut(#[from] tokio::time::error::Elapsed),
}

// Messages ***************************

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub enum Command {
    G15,
    Status,
    Say(Arc<str>),
    SayTeam(Arc<str>),
    Kick {
        /// The uid of the player as returned by [Command::Status] or [Command::G15]
        player: Arc<str>,
        #[serde(default)]
        reason: KickReason,
    },
    Custom(Arc<str>),
}
impl<S> StateUpdater<S> for Command {}

pub enum CommandResponse {
    Connection(Result<Connection<TcpStream>, CommandManagerError>),
    Command(Result<Arc<str>, CommandManagerError>),
}
impl<S> StateUpdater<S> for CommandResponse {}

// Handlers ****************************

pub struct CommandManager {
    rcon: Option<Arc<Mutex<Connection<TcpStream>>>>,
}

impl CommandManager {
    pub fn new() -> CommandManager {
        CommandManager { rcon: None }
    }
}

impl<S, IM, OM> HandlerStruct<S, IM, OM> for CommandManager
where
    IM: Is<Command>,
    OM: Is<CommandResponse>,
{
    fn handle_message(&mut self, state: &S, message: &IM) -> Option<event_loop::Handled<OM>> {
        let command = try_get::<Command>(message)?;

        None
    }
}
