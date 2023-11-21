use std::time::Duration;

use anyhow::{Context, Result};
use rcon::Connection;

use tokio::{
    net::TcpStream,
    sync::mpsc::{error::TryRecvError, unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::timeout,
};

use super::Command;

enum CommandManagerMessage {
    RunCommand(Command),
    SetRconPassword(String),
}

#[derive(Clone)]
pub struct CommandSender {
    sender: UnboundedSender<CommandManagerMessage>,
}

impl CommandSender {
    pub fn run_command(&self, command: Command) {
        self.sender
            .send(CommandManagerMessage::RunCommand(command))
            .expect("Command loop ded");
    }
}

pub struct CommandManager {
    sender: UnboundedSender<CommandManagerMessage>,
    receiver: UnboundedReceiver<String>,
}

impl CommandManager {
    pub async fn new(rcon_password: String) -> CommandManager {
        let (req_tx, resp_rx) = CommandManagerInner::new(rcon_password).await;

        CommandManager {
            sender: req_tx,
            receiver: resp_rx,
        }
    }

    pub fn get_command_sender(&self) -> CommandSender {
        CommandSender {
            sender: self.sender.clone(),
        }
    }

    pub fn run_command(&self, command: Command) {
        self.sender
            .send(CommandManagerMessage::RunCommand(command))
            .expect("Command loop ded");
    }

    pub fn set_rcon_password(&self, rcon_password: String) {
        self.sender
            .send(CommandManagerMessage::SetRconPassword(rcon_password))
            .expect("Command loop ded");
    }

    /// Attempts to receive the next response. Returns [None] if there is none ready
    pub fn try_next_response(&mut self) -> Option<String> {
        match self.receiver.try_recv() {
            Ok(response) => Some(response),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => panic!("Command loop ded"),
        }
    }

    /// Receives the next response. Since this just reading from a [tokio::mpsc::UnboundedReceiver]
    /// it is cancellation safe.
    pub async fn next_response(&mut self) -> String {
        self.receiver.recv().await.expect("Command loop ded")
    }
}

struct CommandManagerInner {
    rcon_password: String,
    rcon: Option<Connection<TcpStream>>,

    request_recv: UnboundedReceiver<CommandManagerMessage>,
    response_send: UnboundedSender<String>,
}

#[allow(dead_code)]
impl CommandManagerInner {
    async fn new(
        rcon_password: String,
    ) -> (
        UnboundedSender<CommandManagerMessage>,
        UnboundedReceiver<String>,
    ) {
        let (req_tx, req_rx) = unbounded_channel();
        let (resp_tx, resp_rx) = unbounded_channel();

        let mut inner = CommandManagerInner {
            rcon_password,
            rcon: None,
            request_recv: req_rx,
            response_send: resp_tx,
        };

        tokio::task::spawn(async move {
            inner.command_loop().await;
        });

        (req_tx, resp_rx)
    }

    async fn command_loop(&mut self) {
        loop {
            match self.request_recv.recv().await.expect("IO loop ded") {
                CommandManagerMessage::RunCommand(cmd) => {
                    let cmd = format!("{}", cmd);
                    if let Err(e) = self.run_command(&cmd).await {
                        tracing::error!("Failed to run command {}: {:?}", cmd, e);
                    }
                }
                CommandManagerMessage::SetRconPassword(password) => {
                    self.rcon_password = password;
                    if let Err(e) = self.try_reconnect().await {
                        tracing::error!("Failed to reconnect to rcon: {:?}", e);
                    }
                }
            }
        }
    }

    pub async fn run_command(&mut self, command: &str) -> Result<()> {
        let rcon = if let Some(rcon) = self.rcon.as_mut() {
            rcon
        } else {
            self.try_reconnect()
                .await
                .context("Failed to reconnect to RCon.")?
        };

        tracing::debug!("Running command \"{}\"", command);
        let result = rcon
            .cmd(command)
            .await
            .map_err(|e| {
                self.rcon = None;
                e
            })
            .context("Failed to run command")?;

        self.response_send
            .send(result)
            .expect("Couldn't send command response");

        Ok(())
    }

    async fn try_reconnect(&mut self) -> Result<&mut Connection<TcpStream>> {
        tracing::debug!("Attempting to reconnect to RCon");
        match timeout(
            Duration::from_secs(2),
            Connection::connect("127.0.0.1:27015", &self.rcon_password),
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
