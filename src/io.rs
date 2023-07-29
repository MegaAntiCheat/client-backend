use anyhow::Context;
use regex::Regex;

use anyhow::{anyhow, Result};
use std::fmt::Display;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender};

use crate::state::State;

use self::command_manager::{CommandManager, KickReason};
use self::filewatcher::FileWatcher;
use self::g15::{G15Parser, G15Player};
use self::regexes::{
    ChatMessage, Hostname, Map, PlayerCount, PlayerKill, ServerIP, StatusLine, REGEX_CHAT,
    REGEX_HOSTNAME, REGEX_IP, REGEX_KILL, REGEX_MAP, REGEX_PLAYERCOUNT, REGEX_STATUS,
};

pub mod command_manager;
pub mod filewatcher;
pub mod g15;
pub mod regexes;

// Enums

#[derive(Debug)]
pub enum IOOutput {
    NoOutput,
    Status(StatusLine),
    MultiStatus(Vec<StatusLine>),
    Chat(ChatMessage),
    Kill(PlayerKill),
    Hostname(Hostname),
    ServerIP(ServerIP),
    Map(Map),
    PlayerCount(PlayerCount),
    G15(Vec<G15Player>),
}

#[allow(dead_code)]
pub enum Commands {
    G15,
    Status,
    Kick(Arc<str>, KickReason),
    Say(String),
}
impl Display for Commands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Commands::G15 => f.write_str("g15_dumpplayer"),
            Commands::Status => f.write_str("status"),
            Commands::Kick(player, reason) => write!(f, "callvote kick \"{} {}\"", player, reason),
            Commands::Say(message) => write!(f, "say \"{}\"", message),
        }
    }
}

// IOThread

pub struct IOManager {
    command_recv: UnboundedReceiver<Commands>,
    command_send: UnboundedSender<Commands>,
    command_manager: CommandManager,
    log_watcher: Option<FileWatcher>,
    parser: G15Parser,
    regex_status: Regex,
    regex_chat: Regex,
    regex_kill: Regex,
    regex_hostname: Regex,
    regex_ip: Regex,
    regex_map: Regex,
    regex_playercount: Regex,
}

impl IOManager {
    pub fn new() -> IOManager {
        let command_manager = CommandManager::new();
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();

        IOManager {
            command_recv: rx,
            command_send: tx,
            command_manager,
            log_watcher: None,
            parser: G15Parser::new(),
            regex_status: Regex::new(REGEX_STATUS).expect("Compile static regex"),
            regex_chat: Regex::new(REGEX_CHAT).expect("Compile static regex"),
            regex_kill: Regex::new(REGEX_KILL).expect("Compile static regex"),
            regex_hostname: Regex::new(REGEX_HOSTNAME).expect("Compile static regex"),
            regex_ip: Regex::new(REGEX_IP).expect("Compile static regex"),
            regex_map: Regex::new(REGEX_MAP).expect("Compile static regex"),
            regex_playercount: Regex::new(REGEX_PLAYERCOUNT).expect("Compile static regex"),
        }
    }

    pub fn get_command_requester(&self) -> UnboundedSender<Commands> {
        self.command_send.clone()
    }

    pub async fn handle_waiting_command(&mut self) -> Result<IOOutput> {
        if let Ok(Some(command)) =
            tokio::time::timeout(Duration::from_millis(50), self.command_recv.recv()).await
        {
            return self.handle_command(command).await;
        }

        Ok(IOOutput::NoOutput)
    }

    /// Run a command and handle the response from it
    pub async fn handle_command(&mut self, command: Commands) -> Result<IOOutput> {
        let resp: String = self
            .command_manager
            .run_command(&format!("{}", command))
            .await
            .context("Failed to run command")?;
        Ok(match command {
            Commands::G15 => {
                let players = self.parser.parse_g15(&resp);
                IOOutput::G15(players)
            }
            Commands::Status => {
                let mut status_lines = Vec::new();
                for l in resp.lines() {
                    if let Some(status_resp) = self.regex_status.captures(l).map(StatusLine::parse)
                    {
                        match status_resp {
                            Ok(status) => status_lines.push(status),
                            Err(e) => {
                                tracing::error!("Error parsing status line: {:?}", e);
                            }
                        }
                    }
                }
                IOOutput::MultiStatus(status_lines)
            }
            Commands::Kick(_, _) | Commands::Say(_) => {
                IOOutput::NoOutput // No return from these other commands.
            }
        })
    }

    /// Parse all of the new log entries that have been written
    pub fn handle_log(&mut self) -> Result<IOOutput> {
        if self.log_watcher.as_ref().is_none() {
            self.reopen_log().context("Failed to reopen log file.")?;
        }

        loop {
            match self
                .log_watcher
                .as_mut()
                .ok_or(anyhow!("Failed to read lines from log file."))?
                .get_line()
            {
                Ok(None) => break,
                Ok(Some(line)) => {
                    // Match status
                    if let Some(caps) = self.regex_status.captures(&line) {
                        match StatusLine::parse(caps) {
                            Ok(status) => return Ok(IOOutput::Status(status)),
                            Err(e) => tracing::error!("Error parsing status line: {:?}", e),
                        }
                        continue;
                    }
                    // Match chat message
                    if let Some(caps) = self.regex_chat.captures(&line) {
                        let chat = ChatMessage::parse(caps);
                        return Ok(IOOutput::Chat(chat));
                    }
                    // Match player kills
                    if let Some(caps) = self.regex_kill.captures(&line) {
                        let kill = PlayerKill::parse(caps);
                        return Ok(IOOutput::Kill(kill));
                    }
                    // Match server hostname
                    if let Some(caps) = self.regex_hostname.captures(&line) {
                        let hostname = Hostname::parse(caps);
                        return Ok(IOOutput::Hostname(hostname));
                    }
                    // Match server IP
                    if let Some(caps) = self.regex_ip.captures(&line) {
                        let ip = ServerIP::parse(caps);
                        return Ok(IOOutput::ServerIP(ip));
                    }
                    // Match server map
                    if let Some(caps) = self.regex_map.captures(&line) {
                        let map = Map::parse(caps);
                        return Ok(IOOutput::Map(map));
                    }
                    // Match server player count
                    if let Some(caps) = self.regex_playercount.captures(&line) {
                        let playercount = PlayerCount::parse(caps);
                        return Ok(IOOutput::PlayerCount(playercount));
                    }
                }
                Err(e) => {
                    self.log_watcher = None;
                    tracing::error!("Failed to read log line: {:?}", e);
                }
            }
        }

        Ok(IOOutput::NoOutput)
    }

    /// Attempt to reopen the log file with the currently set directory.
    fn reopen_log(&mut self) -> Result<()> {
        let state = State::read_state();
        let mut dir: PathBuf = state.settings.get_tf2_directory().into();
        dir.push("tf/console.log");

        match FileWatcher::new(dir) {
            Ok(lw) => {
                self.log_watcher = Some(lw);
                tracing::info!("Successfully opened log file.");
                Ok(())
            }
            Err(e) => {
                self.log_watcher = None;
                Err(e)
            }
        }
    }
}
