use std::{
    fmt::{Debug, Display},
    sync::Arc,
};

use event_loop::{try_get, Handled, HandlerStruct, Is, StateUpdater};
use rcon::Connection;
use serde::Deserialize;
use thiserror::Error;
use tokio::{net::TcpStream, sync::Mutex};

use super::Refresh;

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

impl Debug for CommandResponse {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CommandResponse::Connection(Ok(_)) => {
                write!(f, "CommandResponse(Connection(Connected!))")
            }
            CommandResponse::Connection(Err(e)) => {
                write!(f, "CommandResponse(Connection(Err({:?})))", e)
            }
            CommandResponse::Command(c) => write!(f, "CommandResponse(Command({:?}))", c),
        }
    }
}

// Handlers ****************************

pub struct CommandManager {
    inner: Arc<Mutex<CommandManagerInner>>,
    refresh_status: bool,
}

struct CommandManagerInner {
    pub connection: Option<Connection<TcpStream>>,
}

impl CommandManagerInner {
    fn new() -> CommandManagerInner {
        CommandManagerInner { connection: None }
    }
}

impl CommandManager {
    pub fn new() -> CommandManager {
        CommandManager {
            inner: Arc::new(Mutex::new(CommandManagerInner::new())),
            refresh_status: false,
        }
    }

    fn run_command<OM: Is<CommandResponse>>(&mut self, command: &Command) -> Option<Handled<OM>> {
        // TODO - Run command

        None
    }
}

impl<S, IM, OM> HandlerStruct<S, IM, OM> for CommandManager
where
    IM: Is<Command> + Is<Refresh>,
    OM: Is<CommandResponse>,
{
    fn handle_message(&mut self, state: &S, message: &IM) -> Option<event_loop::Handled<OM>> {
        if let Some(_) = try_get::<Refresh>(message) {
            self.refresh_status = !self.refresh_status;
            if self.refresh_status {
                return self.run_command(&Command::Status);
            } else {
                return self.run_command(&Command::G15);
            }
        }

        self.run_command(try_get::<Command>(message)?)
    }
}
