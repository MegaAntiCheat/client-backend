use regex::Regex;

use serde::Deserialize;
use std::fmt::Display;
use std::path::PathBuf;
use tokio::sync::mpsc::error::TryRecvError;

use tokio::sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender};

use self::command_manager::{CommandManager, CommandSender};
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
    Status(StatusLine),
    Chat(ChatMessage),
    Kill(PlayerKill),
    Hostname(Hostname),
    ServerIP(ServerIP),
    Map(Map),
    PlayerCount(PlayerCount),
    G15(Vec<G15Player>),
}

#[derive(Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub enum Command {
    G15,
    Status,
    Say(String),
    SayTeam(String),
    Kick {
        /// The uid of the player as returned by [Command::Status] or [Command::G15]
        player: String,
        #[serde(default)]
        reason: KickReason,
    },
    Custom(String),
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum KickReason {
    None,
    Idle,
    Cheating,
    Scamming,
}

// IOThread

enum IOManagerMessage {
    SetLogFilePath(PathBuf),
    SetRconPassword(String),
}

/// A thread-safe and cloneable [UnboundedSender] which can be user to send IO updates or run commands
#[derive(Clone)]
pub struct IOSender {
    sender: UnboundedSender<IOManagerMessage>,
    command_sender: CommandSender,
}

impl IOSender {
    /// Get a copy of a [CommandSender], which is much like an [IOSender] but is limited to only running commands.
    pub fn get_command_sender(&self) -> CommandSender {
        self.command_sender.clone()
    }

    /// Request for a console command to be run in-game asynchronously. Eventually the result
    /// of this command can be read with [IOManager::next_io_output] or [IOManager::try_next_io_output] if it was successful.
    pub fn run_command(&self, cmd: Command) {
        self.command_sender.run_command(cmd);
    }

    /// Set the path of the `console.log` file to watch
    pub fn set_file_path(&self, path: PathBuf) {
        self.sender
            .send(IOManagerMessage::SetLogFilePath(path))
            .expect("IO manager ded");
    }

    /// Set the password used by rcon to connect to the game and send console commands.
    pub fn set_rcon_password(&self, password: String) {
        self.sender
            .send(IOManagerMessage::SetRconPassword(password))
            .expect("IO manager ded");
    }
}

/// [IOManager] provides and interface to asynchronously watch the `console.log` file for updates and run
/// commands in the in-game console. Such updates to `console.log` and output from the run commands
/// can be received through [Self::next_io_output] and [Self::try_next_io_output].
///
/// The IOManager can also be used to update the configuration of the internal [FileWatcher] and [CommandManager]
/// such as setting the file path and rcon password.
pub struct IOManager {
    sender: UnboundedSender<IOManagerMessage>,
    receiver: UnboundedReceiver<Vec<IOOutput>>,

    command_sender: CommandSender,
}

impl IOManager {
    /// Create a new [IOManager] which internally consists of a [FileWatcher] and [CommandManager] to watch the given
    /// `console.log` file at `log_file_path`, and send commands over rcon to `localhost` using `rcon_password`
    pub async fn new(log_file_path: PathBuf, rcon_password: String) -> IOManager {
        let command_manager = CommandManager::new(rcon_password).await;
        let file_watcher = FileWatcher::new(log_file_path).await;
        let command_sender = command_manager.get_command_sender();

        let (req_tx, resp_rx) = IOManagerInner::new(command_manager, file_watcher).await;

        IOManager {
            sender: req_tx,
            receiver: resp_rx,
            command_sender,
        }
    }

    /// Get a copy of the [UnboundedSender] which can be used to send IO updates and run commands
    pub fn get_io_sender(&self) -> IOSender {
        IOSender {
            sender: self.sender.clone(),
            command_sender: self.command_sender.clone(),
        }
    }

    /// Get a copy of the [UnboundedSender] which can be used to run commands
    pub fn get_command_sender(&self) -> CommandSender {
        self.command_sender.clone()
    }

    /// Request for a console command to be run in-game asynchronously. Eventually the result
    /// of this command can be read with [Self::next_io_output] or [Self::try_next_io_output] if it was successful.
    pub fn run_command(&self, cmd: Command) {
        self.command_sender.run_command(cmd);
    }

    /// Set the path of the `console.log` file to watch
    pub fn set_file_path(&self, path: PathBuf) {
        self.sender
            .send(IOManagerMessage::SetLogFilePath(path))
            .expect("IO manager ded");
    }

    /// Set the password used by rcon to connect to the game and send console commands.
    pub fn set_rcon_password(&self, password: String) {
        self.sender
            .send(IOManagerMessage::SetRconPassword(password))
            .expect("IO manager ded");
    }

    /// Attempts to receive the next io output. Returns [None] if there is not one ready.
    pub fn try_next_io_output(&mut self) -> Option<Vec<IOOutput>> {
        match self.receiver.try_recv() {
            Ok(out) => Some(out),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => panic!("IO manager ded"),
        }
    }

    /// Receives the next io output. Since this just reading from a [UnboundedReceiver]
    /// it is cancellation safe.
    pub async fn next_io_output(&mut self) -> Vec<IOOutput> {
        self.receiver.recv().await.expect("IO manager ded")
    }
}

struct IOManagerInner {
    command_manager: CommandManager,
    file_watcher: FileWatcher,

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

impl IOManagerInner {
    async fn new(
        command_manager: CommandManager,
        file_watcher: FileWatcher,
    ) -> (
        UnboundedSender<IOManagerMessage>,
        UnboundedReceiver<Vec<IOOutput>>,
    ) {
        let (req_tx, req_rx) = unbounded_channel();
        let (resp_tx, resp_rx) = unbounded_channel();

        let mut inner = IOManagerInner {
            command_manager,
            file_watcher,

            message_recv: req_rx,
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

        tokio::task::spawn(async move {
            inner.io_loop().await;
        });

        (req_tx, resp_rx)
    }

    async fn io_loop(&mut self) {
        loop {
            tokio::select! {
                message = self.message_recv.recv() => {
                    self.handle_message(message.expect("Main loop ded"));
                },
                command_response = self.command_manager.next_response() => {
                    let out = self.read_command_response(command_response);
                    if out.len() > 0 {
                        self.response_send.send(out).expect("Main loop ded");
                    }
                },
                log_line = self.file_watcher.next_line() => {
                    if let Some(out) = self.read_log_line(&log_line) {
                        self.response_send.send(vec![out]).expect("Main loop ded");
                    }
                }
            }
        }
    }

    fn handle_message(&mut self, message: IOManagerMessage) {
        match message {
            IOManagerMessage::SetLogFilePath(path) => self.file_watcher.set_watched_file(path),
            IOManagerMessage::SetRconPassword(password) => {
                self.command_manager.set_rcon_password(password)
            }
        }
    }

    fn read_command_response(&self, response: String) -> Vec<IOOutput> {
        let mut out = Vec::new();

        // Parse out anything from status
        for l in response.lines() {
            if let Some(line) = self.read_log_line(l) {
                out.push(line);
            }
        }

        // Check for G15 output
        let players = self.parser.parse_g15(&response);
        if players.len() > 0 {
            out.push(IOOutput::G15(players));
        }

        out
    }

    fn read_log_line(&self, line: &str) -> Option<IOOutput> {
        // Match status
        if let Some(caps) = self.regex_status.captures(&line) {
            match StatusLine::parse(caps) {
                Ok(status) => return Some(IOOutput::Status(status)),
                Err(e) => tracing::error!("Error parsing status line: {:?}", e),
            }
        }
        // Match chat message
        if let Some(caps) = self.regex_chat.captures(&line) {
            let chat = ChatMessage::parse(caps);
            return Some(IOOutput::Chat(chat));
        }
        // Match player kills
        if let Some(caps) = self.regex_kill.captures(&line) {
            let kill = PlayerKill::parse(caps);
            return Some(IOOutput::Kill(kill));
        }
        // Match server hostname
        if let Some(caps) = self.regex_hostname.captures(&line) {
            let hostname = Hostname::parse(caps);
            return Some(IOOutput::Hostname(hostname));
        }
        // Match server IP
        if let Some(caps) = self.regex_ip.captures(&line) {
            let ip = ServerIP::parse(caps);
            return Some(IOOutput::ServerIP(ip));
        }
        // Match server map
        if let Some(caps) = self.regex_map.captures(&line) {
            let map = Map::parse(caps);
            return Some(IOOutput::Map(map));
        }
        // Match server player count
        if let Some(caps) = self.regex_playercount.captures(&line) {
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
