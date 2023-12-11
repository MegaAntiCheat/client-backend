use std::{sync::Arc, time::Duration};

use anyhow::{Context, Result};
use rcon::Connection;

use tokio::{
    net::TcpStream,
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
    time::timeout,
};

use super::Command;

pub enum CommandManagerMessage {
    RunCommand(Command),
    SetRconPassword(Arc<str>),
}

pub struct CommandManager {
    rcon_password: Arc<str>,
    rcon: Option<Connection<TcpStream>>,

    request_recv: UnboundedReceiver<CommandManagerMessage>,
    response_send: UnboundedSender<Arc<str>>,
}

#[allow(dead_code)]
impl CommandManager {
    pub fn new(
        rcon_password: Arc<str>,
        recv: UnboundedReceiver<CommandManagerMessage>,
    ) -> (UnboundedReceiver<Arc<str>>, CommandManager) {
        let (resp_tx, resp_rx) = unbounded_channel();

        let inner = CommandManager {
            rcon_password,
            rcon: None,
            request_recv: recv,
            response_send: resp_tx,
        };

        (resp_rx, inner)
    }

    /// Start the command manager loop. This will block until the channel is closed, so usually it should be spawned in a separate `tokio::task`
    pub async fn command_loop(&mut self) {
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
            .context("Failed to run command")?
            .into();

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
