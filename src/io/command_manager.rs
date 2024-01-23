use rcon::Connection;
use std::io::ErrorKind;
use std::{sync::Arc, time::Duration};
use tokio::{
    net::TcpStream,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::timeout,
};

use crate::events::command_manager::CommandManagerError;

use super::Command;

/// Since we only _really_ care about differentiating the Rcon errors, those are the values we check more explicitly.
/// For the timeout variant, comparing that they are both the same supertype is simply enough as they only downcast to
/// one error type.
impl PartialEq for CommandManagerError {
    fn eq(&self, other: &Self) -> bool {
        #[allow(clippy::match_like_matches_macro)]
        match (self, other) {
            (Self::Rcon(rcon::Error::Auth), Self::Rcon(rcon::Error::Auth)) => true,
            (Self::Rcon(rcon::Error::CommandTooLong), Self::Rcon(rcon::Error::CommandTooLong)) => {
                true
            }
            (Self::Rcon(rcon::Error::Io(lh)), Self::Rcon(rcon::Error::Io(rh))) => {
                lh.kind() == rh.kind()
            }
            (Self::TimeOut(_), Self::TimeOut(_)) => true,
            _ => false,
        }
    }
}

/// On app launch, the connection error state for RCon will be initialised to 'Never'. Once we have achieved the first connection
/// with the defined RCon properties, we can only ever have an error state of 'Okay' or Current(CommandManagerError)
#[derive(PartialEq)]

enum ErrorState {
    /// Never had an error, never been connected to RCon
    Never,
    /// Currently connected to RCon, logged no error state
    Okay,
    /// No longer or never was connected to RCon due to the wrapped error
    Current(CommandManagerError),
}

pub enum CommandManagerMessage {
    RunCommand(Command),
    SetRconPassword(Arc<str>),
    SetRconPort(u16),
}

pub struct CommandManager {
    rcon_password: Arc<str>,
    rcon: Option<Connection<TcpStream>>,
    rcon_port: u16,
    current_err_state: ErrorState,
    previous_err_state: ErrorState,
    request_recv: UnboundedReceiver<CommandManagerMessage>,
    response_send: UnboundedSender<Arc<str>>,
}

impl CommandManager {
    pub fn new(
        rcon_password: Arc<str>,
        rcon_port: u16,
        recv: UnboundedReceiver<CommandManagerMessage>,
    ) -> (UnboundedReceiver<Arc<str>>, CommandManager) {
        let (resp_tx, resp_rx) = unbounded_channel();

        let inner = CommandManager {
            rcon_password,
            rcon: None,
            rcon_port,
            current_err_state: ErrorState::Never,
            previous_err_state: ErrorState::Never,
            request_recv: recv,
            response_send: resp_tx,
        };

        (resp_rx, inner)
    }

    /// Start the command manager loop. This will block until the channel is closed, so usually it should be spawned in a separate `tokio::task`
    pub async fn command_loop(&mut self) {
        loop {
            // Maintain an error state and historical view for state-based error reporting
            // This avoids reporting the same error message multiple times, but makes sure different messages of the same super type
            // are reported.
            if self.current_err_state != self.previous_err_state {
                match &self.current_err_state {
                    // If we have just launched/reset RCon state, and we get connection refused, just warn about it instead as TF2 likely isn't open
                    ErrorState::Current(e @ CommandManagerError::Rcon(rcon::Error::Io(err)))
                        if self.previous_err_state == ErrorState::Never
                            && err.kind() == ErrorKind::ConnectionRefused =>
                    {
                        tracing::warn!("{} (This is expected behaviour if TF2 is not open)", e)
                    }
                    // We have entered an error state from some other state, or the error state has changed. Report it!
                    ErrorState::Current(err) => {
                        tracing::error!("{}", err);
                    }
                    // We are either okay or never connected. Nothing to report.
                    _ => {}
                };
            }
            // When we report any state other than okay, we always try and reconnect with the current RCon config,
            // except for when we are receiving auth failures.
            match self.current_err_state {
                // if current error state indicates bad auth, don't try and reconnect else we get shunted by TF2
                // When the user fixes their rcon_password in the mac client, it will reset the error state to Never.
                // Known issue: if the user changes the rcon_password _in TF2_, this will not trigger an ErrorState change here.
                ErrorState::Okay
                | ErrorState::Current(CommandManagerError::Rcon(rcon::Error::Auth)) => {}
                // Any other issue is worthy of a reconnect attempt.
                _ => {
                    match self.try_reconnect().await {
                        Ok(_) => {
                            // Current error state (which was _not_ Okay) now presents a historical view on what the error was
                            // Since we are now connected, if the error state indicates never connected, we can assume first time connect
                            // Otherwise this is a reconnect.
                            match self.current_err_state {
                                ErrorState::Current(_) => {
                                    tracing::info!("Succesfully reconnected to RCon")
                                }
                                ErrorState::Never => {
                                    tracing::info!("Succesfully established a connection with RCon")
                                }
                                _ => {}
                            };
                            std::mem::swap(
                                &mut self.current_err_state,
                                &mut self.previous_err_state,
                            );
                            self.current_err_state = ErrorState::Okay;
                        }
                        Err(e) => {
                            // Moves the current error state into the history, and history into current, then override current with the new error.
                            // This avoids cloning/copying errors by simply moving ownership and dropping scope when not needed.
                            std::mem::swap(
                                &mut self.current_err_state,
                                &mut self.previous_err_state,
                            );
                            self.current_err_state = ErrorState::Current(e);
                        }
                    }
                }
            }

            match self
                .request_recv
                .recv()
                .await
                .expect("The main IO Loop experienced a fatal error.")
            {
                CommandManagerMessage::RunCommand(cmd) => {
                    // Only attempt to run commands if the error state indicates we have a valid RCon client.
                    // This prevents getting shunted by the TF2 client for repeated Auth failures
                    if self.current_err_state == ErrorState::Okay {
                        let cmd = format!("{}", cmd);
                        if let Err(e) = self.run_command(&cmd).await {
                            self.previous_err_state = ErrorState::Okay;
                            self.current_err_state = ErrorState::Current(e);
                        }
                    }
                }
                // Any change to the RCon configurations implicates a new RCon connection that we have never connected
                // to in this 'session', so set state to Never instead of some error state or Okay (as we need to trigger a
                // reconnect, but don't have any errors to report)
                CommandManagerMessage::SetRconPassword(password) => {
                    self.rcon_password = password;
                    self.current_err_state = ErrorState::Never;
                }
                CommandManagerMessage::SetRconPort(port) => {
                    self.rcon_port = port;
                    self.current_err_state = ErrorState::Never;
                }
            }
        }
    }

    pub async fn run_command(&mut self, command: &str) -> Result<(), CommandManagerError> {
        let rcon = self.rcon.as_mut().unwrap();

        tracing::debug!("Running command \"{}\"", command);
        let result = rcon
            .cmd(command)
            .await
            .map_err(|e| {
                self.rcon = None;
                e
            })?
            .into();

        self.response_send.send(result).unwrap();

        Ok(())
    }

    async fn try_reconnect(&mut self) -> Result<(), CommandManagerError> {
        match self.current_err_state {
            ErrorState::Current(_) => tracing::debug!("Attempting to reconnect to RCon"),
            ErrorState::Never => tracing::debug!("Attempting to connect to RCon"),
            _ => {}
        };

        match timeout(
            // Windows will try and connect to an unbound port up to 3 times, with 500ms intervals
            // 2000ms was too little time on the average system to accurately return the 'Connection Refused' error, and
            // would instead return Elapsed.
            Duration::from_millis(2500),
            Connection::connect(
                format!("127.0.0.1:{}", &self.rcon_port),
                &self.rcon_password,
            ),
        )
        .await
        {
            Ok(Ok(con)) => {
                self.rcon = Some(con);
                Ok(())
            }
            Ok(Err(e)) => {
                self.rcon = None;
                Err(e.into())
            }
            Err(e) => {
                self.rcon = None;
                Err(e.into())
            }
        }
    }
}
