use regex::Regex;

use regexes::StatusLine;
use regexes::REGEX_STATUS;
use std::fmt::Display;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc::Receiver;
use tokio::sync::mpsc::Sender;

use crate::state::State;

use self::command_manager::CommandManager;
use self::command_manager::KickReason;
use self::filewatcher::FileWatcher;
use self::g15::G15Parser;
use self::regexes::ChatMessage;
use self::regexes::Hostname;
use self::regexes::Map;
use self::regexes::PlayerCount;
use self::regexes::PlayerKill;
use self::regexes::ServerIP;
use self::regexes::REGEX_CHAT;
use self::regexes::REGEX_HOSTNAME;
use self::regexes::REGEX_IP;
use self::regexes::REGEX_KILL;
use self::regexes::REGEX_MAP;
use self::regexes::REGEX_PLAYERCOUNT;

pub mod command_manager;
pub mod filewatcher;
pub mod g15;
pub mod regexes;

// Enums

#[derive(Debug)]
pub enum IOOutput {
    Status(StatusLine),
    Chat(ChatMessage),
    Kill(PlayerKill),
    Hostname(Hostname),
    ServerIP(ServerIP),
    Map(Map),
    PlayerCount(PlayerCount),
    G15(Vec<g15::G15Player>),
}

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
    command_recv: Receiver<Commands>,
    command_send: Sender<Commands>,
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
        let (tx, rx) = tokio::sync::mpsc::channel(16);

        IOManager {
            command_recv: rx,
            command_send: tx,
            command_manager,
            log_watcher: None,
            parser: G15Parser::new(),
            regex_status: Regex::new(REGEX_STATUS).unwrap(),
            regex_chat: Regex::new(REGEX_CHAT).unwrap(),
            regex_kill: Regex::new(REGEX_KILL).unwrap(),
            regex_hostname: Regex::new(REGEX_HOSTNAME).unwrap(),
            regex_ip: Regex::new(REGEX_IP).unwrap(),
            regex_map: Regex::new(REGEX_MAP).unwrap(),
            regex_playercount: Regex::new(REGEX_PLAYERCOUNT).unwrap(),
        }
    }

    pub fn get_command_requester(&self) -> Sender<Commands> {
        self.command_send.clone()
    }

    pub async fn handle_waiting_command(&mut self) -> Result<Option<IOOutput>, rcon::Error> {
        if let Ok(Some(command)) =
            tokio::time::timeout(Duration::from_millis(50), self.command_recv.recv()).await
        {
            return self.handle_command(command).await;
        }

        Ok(None)
    }

    /// Run a command and handle the response from it
    pub async fn handle_command(
        &mut self,
        command: Commands,
    ) -> Result<Option<IOOutput>, rcon::Error> {
        let resp: String = self
            .command_manager
            .run_command(&format!("{}", command))
            .await?;
        Ok(match command {
            Commands::G15 => {
                let players = self.parser.parse_g15(&resp);
                Some(IOOutput::G15(players))
            }
            Commands::Kick(_, _) => {
                None // No return from a kick invocation.
            }
            Commands::Status => {
                None // No return via RCON for status.
            }
            Commands::Say(_) => {
                None // No return from a say invocation.
            }
        })
    }

    /// Parse all of the new log entries that have been written
    pub fn handle_log(&mut self) -> std::io::Result<Option<IOOutput>> {
        if self.log_watcher.as_ref().is_none() {
            self.reopen_log()?;
        }

        while let Some(line) = self.log_watcher.as_mut().unwrap().get_line() {
            // Match status
            if let Some(caps) = self.regex_status.captures(&line) {
                match StatusLine::parse(caps) {
                    Ok(status) => return Ok(Some(IOOutput::Status(status))),
                    Err(e) => log::error!("Malformed steamid: {}", e),
                }
                continue;
            }
            // Match chat message
            if let Some(caps) = self.regex_chat.captures(&line) {
                let chat = ChatMessage::parse(caps);
                return Ok(Some(IOOutput::Chat(chat)));
            }
            // Match player kills
            if let Some(caps) = self.regex_kill.captures(&line) {
                let kill = PlayerKill::parse(caps);
                return Ok(Some(IOOutput::Kill(kill)));
            }
            // Match server hostname
            if let Some(caps) = self.regex_hostname.captures(&line) {
                let hostname = Hostname::parse(caps);
                return Ok(Some(IOOutput::Hostname(hostname)));
            }
            // Match server IP
            if let Some(caps) = self.regex_ip.captures(&line) {
                let ip = ServerIP::parse(caps);
                return Ok(Some(IOOutput::ServerIP(ip)));
            }
            // Match server map
            if let Some(caps) = self.regex_map.captures(&line) {
                let map = Map::parse(caps);
                return Ok(Some(IOOutput::Map(map)));
            }
            // Match server player count
            if let Some(caps) = self.regex_playercount.captures(&line) {
                let playercount = PlayerCount::parse(caps);
                return Ok(Some(IOOutput::PlayerCount(playercount)));
            }
        }

        Ok(None)
    }

    /// Attempt to reopen the log file with the currently set directory.
    /// If the log file fails to be opened, an [LogOutput::NoLogFile] is sent back to the main thread and [Self::log_watcher] is set to [None]
    fn reopen_log(&mut self) -> std::io::Result<()> {
        let state = State::read_state();
        let dir = state.settings.get_tf2_directory();

        match FileWatcher::use_directory(dir.into()) {
            Ok(lw) => {
                log::debug!("Successfully opened log file");
                self.log_watcher = Some(lw);
                Ok(())
            }
            Err(e) => {
                self.log_watcher = None;
                log::error!("Failed to open log file: {:?}", e);
                Err(e)
            }
        }
    }
}
