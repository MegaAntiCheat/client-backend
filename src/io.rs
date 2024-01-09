use regex::Regex;

use serde::Deserialize;
use std::fmt::Display;
use std::path::PathBuf;
use std::sync::Arc;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use self::command_manager::{CommandManager, CommandManagerMessage};
use self::filewatcher::{FileWatcher, FileWatcherCommand};
use self::g15::{G15Parser, G15Player};
use self::regexes::{
    ChatMessage, Hostname, Map, PlayerCount, PlayerKill, ServerIP, StatusLine, REGEX_CHAT,
    REGEX_HOSTNAME, REGEX_IP, REGEX_KILL, REGEX_MAP, REGEX_PLAYERCOUNT, REGEX_STATUS,
};

pub mod command_manager;
pub mod filewatcher;
pub mod g15;
pub(crate) mod regexes;

// Enums

#[derive(Debug, Clone)]
pub enum IOOutput {
    Status(StatusLine),
    Chat(ChatMessage),
    Kill(PlayerKill),
    Hostname(Hostname),
    ServerIP(ServerIP),
    Map(Map),
    PlayerCount(PlayerCount),
    G15(Vec<G15Player>),
}

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

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub enum KickReason {
    None,
    Idle,
    Cheating,
    Scamming,
}

// IOThread

#[derive(Debug, Clone)]
pub enum IOManagerMessage {
    SetLogFilePath(PathBuf),
    SetRconPassword(Arc<str>),
    SetRconPort(u16),
    RunCommand(Command),
}

pub struct IOManager {
    command: Option<CommandManager>,
    command_send: UnboundedSender<CommandManagerMessage>,
    command_recv: UnboundedReceiver<Arc<str>>,

    filewatcher: Option<FileWatcher>,
    filewatcher_send: UnboundedSender<FileWatcherCommand>,
    filewatcher_recv: UnboundedReceiver<Arc<str>>,

    message_recv: UnboundedReceiver<IOManagerMessage>,
    response_send: UnboundedSender<Vec<IOOutput>>,

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
    pub fn new(
        log_file_path: PathBuf,
        rcon_password: Arc<str>,
        rcon_port: u16,
        recv: UnboundedReceiver<IOManagerMessage>,
    ) -> (UnboundedReceiver<Vec<IOOutput>>, IOManager) {
        let (resp_tx, resp_rx) = unbounded_channel();

        let (command_send, command_recv) = unbounded_channel();
        let (command_recv, command_manager) =
            CommandManager::new(rcon_password, rcon_port, command_recv);

        let (filewatcher_send, filewatcher_recv) = unbounded_channel();
        let (filewatcher_recv, file_watcher) = FileWatcher::new(log_file_path, filewatcher_recv);

        let inner = IOManager {
            command: Some(command_manager),
            command_send,
            command_recv,

            filewatcher: Some(file_watcher),
            filewatcher_send,
            filewatcher_recv,

            message_recv: recv,
            response_send: resp_tx,

            parser: G15Parser::new(),
            regex_status: Regex::new(REGEX_STATUS).expect("Compile static regex"),
            regex_chat: Regex::new(REGEX_CHAT).expect("Compile static regex"),
            regex_kill: Regex::new(REGEX_KILL).expect("Compile static regex"),
            regex_hostname: Regex::new(REGEX_HOSTNAME).expect("Compile static regex"),
            regex_ip: Regex::new(REGEX_IP).expect("Compile static regex"),
            regex_map: Regex::new(REGEX_MAP).expect("Compile static regex"),
            regex_playercount: Regex::new(REGEX_PLAYERCOUNT).expect("Compile static regex"),
        };

        (resp_rx, inner)
    }

    /// Start the IO manager loop. This will block until the channel is closed, so usually it should be spawned in a separate `tokio::task`
    pub async fn io_loop(&mut self) {
        // File watcher
        let mut filewatcher = None;
        std::mem::swap(&mut filewatcher, &mut self.filewatcher);
        tokio::task::spawn(async move {
            filewatcher.unwrap().file_watch_loop().await;
        });

        // Command manager
        let mut command = None;
        std::mem::swap(&mut command, &mut self.command);
        tokio::task::spawn(async move {
            command.unwrap().command_loop().await;
        });

        loop {
            tokio::select! {
                message = self.message_recv.recv() => {
                    self.handle_message(message.expect("Main loop ded"));
                },
                command_response = self.command_recv.recv() => {
                    let out = self.read_command_response(command_response.expect("Failed to receive command response"));
                    if !out.is_empty() {
                        self.response_send.send(out).expect("Main loop ded");
                    }
                },
                log_line = self.filewatcher_recv.recv() => {
                    if let Some(out) = self.read_log_line(&log_line.expect("Failed to receive next file line")) {
                        self.response_send.send(vec![out]).expect("Main loop ded");
                    }
                }
            }
        }
    }

    fn handle_message(&mut self, message: IOManagerMessage) {
        match message {
            IOManagerMessage::SetLogFilePath(path) => self
                .filewatcher_send
                .send(FileWatcherCommand::SetWatchedFile(path))
                .unwrap(),
            IOManagerMessage::SetRconPassword(password) => self
                .command_send
                .send(CommandManagerMessage::SetRconPassword(password))
                .unwrap(),
            IOManagerMessage::SetRconPort(port) => self
                .command_send
                .send(CommandManagerMessage::SetRconPort(port))
                .unwrap(),
            IOManagerMessage::RunCommand(cmd) => self
                .command_send
                .send(CommandManagerMessage::RunCommand(cmd))
                .unwrap(),
        }
    }

    fn read_command_response(&self, response: Arc<str>) -> Vec<IOOutput> {
        let mut out = Vec::new();

        // Parse out anything from status
        for l in response.lines() {
            if let Some(line) = self.read_log_line(l) {
                out.push(line);
            }
        }

        // Check for G15 output
        let players = self.parser.parse_g15(&response);
        if !players.is_empty() {
            out.push(IOOutput::G15(players));
        }

        out
    }

    fn read_log_line(&self, line: &str) -> Option<IOOutput> {
        // Match status
        if let Some(caps) = self.regex_status.captures(line) {
            match StatusLine::parse(caps) {
                Ok(status) => return Some(IOOutput::Status(status)),
                Err(e) => tracing::error!("Error parsing status line: {:?}", e),
            }
        }
        // Match chat message
        if let Some(caps) = self.regex_chat.captures(line) {
            let chat = ChatMessage::parse(caps);
            return Some(IOOutput::Chat(chat));
        }
        // Match player kills
        if let Some(caps) = self.regex_kill.captures(line) {
            let kill = PlayerKill::parse(caps);
            return Some(IOOutput::Kill(kill));
        }
        // Match server hostname
        if let Some(caps) = self.regex_hostname.captures(line) {
            let hostname = Hostname::parse(caps);
            return Some(IOOutput::Hostname(hostname));
        }
        // Match server IP
        if let Some(caps) = self.regex_ip.captures(line) {
            let ip = ServerIP::parse(caps);
            return Some(IOOutput::ServerIP(ip));
        }
        // Match server map
        if let Some(caps) = self.regex_map.captures(line) {
            let map = Map::parse(caps);
            return Some(IOOutput::Map(map));
        }
        // Match server player count
        if let Some(caps) = self.regex_playercount.captures(line) {
            let playercount = PlayerCount::parse(caps);
            return Some(IOOutput::PlayerCount(playercount));
        }

        None
    }
}

// Iterator impl and other convenience stuff we don't need to worry about

impl Display for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Command::G15 => f.write_str("g15_dumpplayer"),
            Command::Status => f.write_str("status"),
            Command::Kick { player, reason } => {
                write!(f, "callvote kick \"{} {}\"", player, reason)
            }
            Command::Say(message) => write!(f, "say \"{}\"", message),
            Command::SayTeam(message) => write!(f, "say_team \"{}\"", message),
            Command::Custom(command) => write!(f, "{}", command),
        }
    }
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
