use anyhow::Result;
use rcon::Connection;
use std::{sync::Arc, time::Duration};
use thiserror::Error;
use tokio::{
    net::TcpStream,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::timeout,
};

use super::Command;

#[derive(Debug, Error)]
pub enum CommandManagerError {
    #[error("RCon error {0}")]
    Rcon(#[from] rcon::Error),
    #[error("Rcon connection timeout: {0}")]
    TimeOut(#[from] tokio::time::error::Elapsed),
    #[error("{0:?}")]
    Other(#[from] anyhow::Error),
}

impl PartialEq for CommandManagerError {
    fn eq(&self, other: &Self) -> bool {
        #[allow(clippy::match_like_matches_macro)]
        match (self, other) {
            (Self::Rcon(rcon::Error::Auth), Self::Rcon(rcon::Error::Auth)) => true,
            (Self::Rcon(rcon::Error::CommandTooLong), Self::Rcon(rcon::Error::CommandTooLong)) => {
                true
            }
            (Self::Rcon(rcon::Error::Io(_)), Self::Rcon(rcon::Error::Io(_))) => true,
            (Self::TimeOut(_), Self::TimeOut(_)) => true,
            (Self::Other(_), Self::Other(_)) => true,
            _ => false,
        }
    }
}

#[derive(PartialEq)]
enum ErrorState {
    Never,
    Okay,
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
    error_state: ErrorState,
    error_hist: ErrorState,
    request_recv: UnboundedReceiver<CommandManagerMessage>,
    response_send: UnboundedSender<Arc<str>>,
}

#[allow(dead_code)]
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
            error_state: ErrorState::Never,
            error_hist: ErrorState::Never,
            request_recv: recv,
            response_send: resp_tx,
        };

        (resp_rx, inner)
    }

    /// Start the command manager loop. This will block until the channel is closed, so usually it should be spawned in a separate `tokio::task`
    pub async fn command_loop(&mut self) {
        // let mut printed_for: ErrorState = ErrorState::Never;
        loop {
            if self.error_state != self.error_hist {
                match &self.error_state {
                    ErrorState::Current(e @ CommandManagerError::TimeOut(_))
                        if self.error_hist == ErrorState::Never =>
                    {
                        tracing::warn!("{}", e)
                    }
                    ErrorState::Current(err) => {
                        tracing::error!("{}", err);
                    }
                    _ => {}
                };
            }

            if self.error_state != ErrorState::Okay {
                match self.try_reconnect().await {
                    Ok(_) => {
                        match self.error_state {
                            ErrorState::Current(_) => {
                                tracing::info!("Succesfully reconnected to RCon")
                            }
                            ErrorState::Never => {
                                tracing::info!("Succesfully established a connection with RCon")
                            }
                            _ => {}
                        };
                        std::mem::swap(&mut self.error_state, &mut self.error_hist);
                        self.error_state = ErrorState::Okay;
                    } // :)
                    Err(e) => {
                        std::mem::swap(&mut self.error_state, &mut self.error_hist);
                        self.error_state = ErrorState::Current(e);
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
                    if self.error_state == ErrorState::Okay {
                        let cmd = format!("{}", cmd);
                        if let Err(e) = self.run_command(&cmd).await {
                            tracing::error!("Failed to run command {}: {:?}", cmd, e);
                            self.error_state = ErrorState::Current(e);
                        }
                    }
                }
                CommandManagerMessage::SetRconPassword(password) => {
                    self.rcon_password = password;
                    self.error_state = ErrorState::Never;
                }
                CommandManagerMessage::SetRconPort(port) => {
                    self.rcon_port = port;
                    self.error_state = ErrorState::Never;
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
        match self.error_state {
            ErrorState::Current(_) => tracing::debug!("Attempting to reconnect to RCon"),
            ErrorState::Never => tracing::debug!("Attempting to connect to RCon"),
            _ => {}
        };

        match timeout(
            Duration::from_secs(2),
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
