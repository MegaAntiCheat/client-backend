use std::{sync::Arc, time::Duration};
use anyhow::ensure;
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
    SetRconPort(u16),
}

pub struct CommandManager {
    rcon_password: Arc<str>,
    rcon: Option<Connection<TcpStream>>,
    rcon_port: u16,
    rcon_connected: bool,
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
            rcon_connected: true, // Assume rcon connected until failure
            request_recv: recv,
            response_send: resp_tx,
        };

        (resp_rx, inner)
    }

    /// Start the command manager loop. This will block until the channel is closed, so usually it should be spawned in a separate `tokio::task`
    pub async fn command_loop(&mut self) {
        let mut error_printed: bool = false;
        loop {
            if !self.rcon_connected {
                if let Err(e) = self.try_reconnect().await {
                    if !error_printed {
                        tracing::error!("Couldn't connect to RCON: {:?}", e);
                        error_printed = true;
                    }
                }
            }
            
            match self.request_recv.recv().await.expect("The main IO Loop experienced a fatal error.") {
                CommandManagerMessage::RunCommand(cmd) => {
                    if self.rcon_connected {
                        let cmd = format!("{}", cmd);
                        if let Err(e) = self.run_command(&cmd).await {
                            tracing::error!("Failed to run command {}: {:?}", cmd, e);
                            self.rcon_connected = false;
                            error_printed = false;
                        }
                    }
                    
                }
                CommandManagerMessage::SetRconPassword(password) => {
                    self.rcon_password = password;
                    self.rcon_connected = false;
                    error_printed = false;
                    
                }
                CommandManagerMessage::SetRconPort(port) => {
                    self.rcon_port = port;
                    self.rcon_connected = false;
                    error_printed = false;
                }
            }
        }
    }

    pub async fn run_command(&mut self, command: &str) -> Result<()> {
        ensure!(self.rcon.as_mut().is_some(), "Couldn't reach RCON (no client).");
        let rcon = self.rcon.as_mut().unwrap();

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
            Connection::connect(
                format!("127.0.0.1:{}", &self.rcon_port),
                &self.rcon_password,
            ),
        )
        .await
        {
            Ok(Ok(con)) => {
                self.rcon = Some(con);
                self.rcon_connected = true;
                tracing::info!("RCon reconnected.");
                Ok(self.rcon.as_mut().expect(""))
            }
            Ok(Err(e)) => {
                self.rcon = None;
                self.rcon_connected = false;
                Err(e).context("Failed to establish connection")
            }
            Err(e) => {
                self.rcon = None;
                self.rcon_connected = false;
                Err(e).context("RCon connection timed out")
            }
        }
    }
}
