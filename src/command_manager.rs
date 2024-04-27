use std::{
    fmt::{Debug, Display},
    io::ErrorKind,
    sync::Arc,
    time::Duration,
};

use event_loop::{try_get, Handled, HandlerStruct, Is};
use rcon::Connection;
use serde::Deserialize;
use thiserror::Error;
use tokio::{net::TcpStream, sync::Mutex, time::timeout};

use super::console::RawConsoleOutput;
use crate::{events::Refresh, player_records::Verdict, state::MACState, player::{PlayerState, Team}};

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
            Self::None => "other",
            Self::Idle => "idle",
            Self::Cheating => "cheating",
            Self::Scamming => "scamming",
        })
    }
}

#[derive(Debug, Error)]
pub enum Error {
    #[error("RCon error {0}")]
    Rcon(#[from] rcon::Error),
    #[error("Rcon connection timeout: {0}")]
    TimeOut(#[from] tokio::time::error::Elapsed),
}

/// Since we only _really_ care about differentiating the Rcon errors, those are
/// the values we check more explicitly. For the timeout variant, comparing that
/// they are both the same supertype is simply enough as they only downcast to
/// one error type.
impl PartialEq for Error {
    fn eq(&self, other: &Self) -> bool {
        #[allow(clippy::match_like_matches_macro)]
        match (self, other) {
            (Self::Rcon(rcon::Error::CommandTooLong), Self::Rcon(rcon::Error::CommandTooLong))
            | (Self::Rcon(rcon::Error::Auth), Self::Rcon(rcon::Error::Auth))
            | (Self::TimeOut(_), Self::TimeOut(_)) => true,
            (Self::Rcon(rcon::Error::Io(lh)), Self::Rcon(rcon::Error::Io(rh))) => {
                lh.kind() == rh.kind()
            }
            _ => false,
        }
    }
}

/// On app launch, the connection error state for `RCon` will be initialised to
/// 'Never'. Once we have achieved the first connection with the defined `RCon`
/// properties, we can only ever have an error state of 'Okay' or
/// Current(CommandManagerError)
#[derive(PartialEq)]
enum ErrorState {
    /// Never had an error, never been connected to RCon
    Never,
    /// Currently connected to RCon, logged no error state
    Okay,
    /// No longer or never was connected to RCon due to the wrapped error
    Current(Error),
}

// Messages ***************************

#[derive(Deserialize, Debug, Clone)]
#[serde(rename_all = "camelCase")]
pub enum Command {
    G15,
    Status,
    Say(String),
    SayTeam(String),
    Kick {
        /// The uid of the player as returned by [Command::Status] or
        /// [Command::G15]
        player: String,
        #[serde(default)]
        reason: KickReason,
    },
    Custom(String),
}

impl Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::G15 => f.write_str("g15_dumpplayer"),
            Self::Status => f.write_str("status"),
            Self::Kick { player, reason } => {
                write!(f, "callvote kick \"{player} {reason}\"")
            }
            Self::Say(message) => write!(f, "say \"{message}\""),
            Self::SayTeam(message) => write!(f, "say_team \"{message}\""),
            Self::Custom(command) => write!(f, "{command}"),
        }
    }
}

// Handlers ****************************

pub struct CommandManager {
    inner: Arc<Mutex<CommandManagerInner>>,
    refresh_status: bool,
}

struct CommandManagerInner {
    connection: Option<Connection<TcpStream>>,
    password: String,
    port: u16,

    current_err_state: ErrorState,
    previous_err_state: ErrorState,
}

impl CommandManagerInner {
    async fn run_command<M: Is<RawConsoleOutput>>(
        &mut self,
        cmd: Command,
        port: u16,
        password: String,
    ) -> Option<M> {
        let needs_reconnect = password != self.password
            || port != self.port
            || match self.current_err_state {
                // Don't try to keep reconnecting on bad auth, otherwise TF2 will shunt the
                // connection for spam
                ErrorState::Current(Error::Rcon(rcon::Error::Auth)) | ErrorState::Okay => false,
                _ => true,
            };

        // Known issue: if the user changes the rcon_password _in TF2_, this will not
        // trigger a reconnect
        if needs_reconnect {
            self.port = port;
            self.password = password;

            match self.try_reconnect().await {
                Ok(()) => {
                    // Current error state now presents a historical view
                    // on what the error was. Since we are now connected, if the error state
                    // indicates never connected, we can assume first time
                    // connect Otherwise this is a reconnect.
                    match self.current_err_state {
                        ErrorState::Current(_) => {
                            tracing::info!("Succesfully reconnected to RCon");
                        }
                        ErrorState::Never => {
                            tracing::info!("Succesfully established a connection with RCon");
                        }
                        ErrorState::Okay => {}
                    };
                    std::mem::swap(&mut self.current_err_state, &mut self.previous_err_state);
                    self.current_err_state = ErrorState::Okay;
                }
                Err(e) => {
                    std::mem::swap(&mut self.current_err_state, &mut self.previous_err_state);
                    self.current_err_state = ErrorState::Current(e);

                    if self.current_err_state != self.previous_err_state {
                        match &self.current_err_state {
                            // If we have just launched/reset RCon state, and we get connection
                            // refused, just warn about it instead as TF2 likely isn't open
                            ErrorState::Current(e @ Error::Rcon(rcon::Error::Io(err)))
                                if self.previous_err_state == ErrorState::Never
                                    && err.kind() == ErrorKind::ConnectionRefused =>
                            {
                                tracing::warn!(
                                    "{e} (This is expected behaviour if TF2 is not open)"
                                );
                            }
                            // We have entered an error state from some other state, or the error
                            // state has changed. Report it!
                            ErrorState::Current(err) => {
                                tracing::error!("{}", err);
                            }
                            _ => {}
                        };
                    }
                }
            }
        }

        if let Some(rcon) = &mut self.connection {
            tracing::debug!("Running command \"{}\"", cmd);
            let result = rcon.cmd(&format!("{cmd}")).await.map_err(|e| {
                self.connection = None;
                e
            });

            match result {
                Ok(out) => {
                    return Some(RawConsoleOutput(out).into());
                }
                Err(e) => {
                    self.previous_err_state = ErrorState::Okay;
                    self.current_err_state = ErrorState::Current(e.into());
                }
            }
        }

        None
    }

    async fn try_reconnect(&mut self) -> Result<(), Error> {
        if self.current_err_state == ErrorState::Never {
            tracing::debug!("Attempting to connect to RCon");
        } else {
            tracing::debug!("Attempting to reconnect to RCon");
        };

        match timeout(
            // Windows will try and connect to an unbound port up to 3 times, with 500ms intervals
            // 2000ms was too little time on the average system to accurately return the
            // 'Connection Refused' error, and would instead return Elapsed.
            Duration::from_millis(2500),
            Connection::connect(format!("127.0.0.1:{}", self.port), &self.password),
        )
        .await
        {
            Ok(Ok(con)) => {
                self.connection = Some(con);
                Ok(())
            }
            Ok(Err(e)) => {
                self.connection = None;
                Err(e.into())
            }
            Err(e) => {
                self.connection = None;
                Err(e.into())
            }
        }
    }
}

impl CommandManagerInner {
    fn new() -> Self {
        Self {
            connection: None,
            current_err_state: ErrorState::Never,
            previous_err_state: ErrorState::Never,
            password: String::new(),
            port: 27015,
        }
    }
}

impl CommandManager {
    #[must_use]
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(CommandManagerInner::new())),
            refresh_status: false,
        }
    }

    fn run_command<OM: Is<RawConsoleOutput>>(
        &mut self,
        command: &Command,
        port: u16,
        password: String,
    ) -> Option<Handled<OM>> {
        let inner = self.inner.clone();
        let cmd = command.clone();
        Handled::future(async move {
            let mut inner = inner.lock().await;
            inner.run_command(cmd, port, password).await
        })
    }
}

impl Default for CommandManager {
    fn default() -> Self {
        Self::new()
    }
}

impl<IM, OM> HandlerStruct<MACState, IM, OM> for CommandManager
where
    IM: Is<Command> + Is<Refresh>,
    OM: Is<RawConsoleOutput>,
{
    fn handle_message(
        &mut self,
        state: &MACState,
        message: &IM,
    ) -> Option<event_loop::Handled<OM>> {
        let port = state.settings.rcon_port();
        let pwd = state.settings.rcon_password();

        if try_get::<Refresh>(message).is_some() {
            self.refresh_status = !self.refresh_status;
            if self.refresh_status {
                return self.run_command(&Command::Status, port, pwd.to_owned());
            }
            return self.run_command(&Command::G15, port, pwd.to_owned());
        }

        self.run_command(try_get::<Command>(message)?, port, pwd.to_owned())
    }
}

pub struct DumbAutoKick;
impl<IM, OM> HandlerStruct<MACState, IM, OM> for DumbAutoKick
where
    IM: Is<Refresh>,
    OM: Is<Command>,
{
    fn handle_message(&mut self, state: &MACState, message: &IM) -> Option<Handled<OM>> {
        let _ = try_get(message)?;
        if !state.settings.autokick_bots() {
            return None;
        }

        let user_team = state
            .players
            .user
            .as_ref()
            .and_then(|s| state.players.game_info.get(s))
            .map(|gi| gi.team)?;

        let to_kick = state
            .players
            .connected
            .iter()
            .filter(|s| {
                state
                    .players
                    .records
                    .get(*s)
                    .is_some_and(|r| r.verdict() == Verdict::Bot)
            })
            .filter_map(|s| state.players.game_info.get(s))
            .filter(|gi| gi.team == user_team && gi.team != Team::Unassigned && gi.state == PlayerState::Active)
            .map(|gi| gi.userid.clone())
            .map(|id| Command::Kick {
                player: id,
                reason: KickReason::Cheating,
            })
            .map(|c| Handled::single(c));

        Handled::multiple(to_kick)
    }
}
