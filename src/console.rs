use std::{path::PathBuf, sync::Arc};

use event_loop::{Handled, HandlerStruct, Is, MessageSource, StateUpdater};
use regex::Regex;
use tokio::sync::mpsc::{error::TryRecvError, UnboundedReceiver};

use crate::{
    io::{
        filewatcher::FileWatcher,
        g15::{G15Player, Parser},
        regexes::{
            ChatMessage, Hostname, Map, PlayerCount, PlayerKill, ServerIP, StatusLine, DemoStop, 
            REGEX_CHAT, REGEX_HOSTNAME, REGEX_IP, REGEX_KILL, REGEX_MAP, REGEX_PLAYERCOUNT, REGEX_STATUS, REGEX_DEMOSTOP
        },
    },
    state::MACState,
};

#[derive(Debug, Clone)]
pub struct RawConsoleOutput(pub Arc<str>);

#[allow(clippy::module_name_repetitions)]
pub struct ConsoleLog {
    pub recv: UnboundedReceiver<Arc<str>>,
    logged_error: bool,
}
impl<M: Is<RawConsoleOutput>> MessageSource<M> for ConsoleLog {
    fn next_message(&mut self) -> Option<M> {
        match self.recv.try_recv() {
            Ok(msg) => Some(RawConsoleOutput(msg).into()),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                if !self.logged_error {
                    tracing::error!("No more console messages coming.");
                    self.logged_error = true;
                }
                None
            }
        }
    }
}
impl ConsoleLog {
    /// # Panics
    /// If tokio fails to spawn the task.
    #[allow(clippy::unused_async)]
    pub async fn new(log_file_path: PathBuf) -> Self {
        let (console_rx, mut log_watcher) = FileWatcher::new(log_file_path);
        tokio::task::spawn(async move {
            log_watcher.file_watch_loop().await;
        });

        Self {
            recv: console_rx,
            logged_error: false,
        }
    }
}

#[derive(Debug, Clone)]
#[allow(clippy::module_name_repetitions)]
pub enum ConsoleOutput {
    Status(StatusLine),
    Chat(ChatMessage),
    Kill(PlayerKill),
    Hostname(Hostname),
    ServerIP(ServerIP),
    Map(Map),
    PlayerCount(PlayerCount),
    G15(Vec<G15Player>),
    DemoStop(DemoStop),
}
impl StateUpdater<MACState> for ConsoleOutput {
    fn update_state(self, state: &mut MACState) {
        state.handle_console_output(self);
    }
}

#[allow(clippy::module_name_repetitions)]
pub struct ConsoleParser {
    g15_parser: Parser,
    regex_status: Regex,
    regex_chat: Regex,
    regex_kill: Regex,
    regex_hostname: Regex,
    regex_ip: Regex,
    regex_map: Regex,
    regex_playercount: Regex,
    regex_demostop: Regex
}

impl Default for ConsoleParser {
    fn default() -> Self {
        Self {
            g15_parser: Parser::new(),
            regex_status: Regex::new(REGEX_STATUS).expect("Compile static regex"),
            regex_chat: Regex::new(REGEX_CHAT).expect("Compile static regex"),
            regex_kill: Regex::new(REGEX_KILL).expect("Compile static regex"),
            regex_hostname: Regex::new(REGEX_HOSTNAME).expect("Compile static regex"),
            regex_ip: Regex::new(REGEX_IP).expect("Compile static regex"),
            regex_map: Regex::new(REGEX_MAP).expect("Compile static regex"),
            regex_playercount: Regex::new(REGEX_PLAYERCOUNT).expect("Compile static regex"),
            regex_demostop: Regex::new(REGEX_DEMOSTOP).expect("Compile static regex"),
        }
    }
}

impl<S, IM, OM> HandlerStruct<S, IM, OM> for ConsoleParser
where
    IM: Is<RawConsoleOutput>,
    OM: Is<ConsoleOutput>,
{
    fn handle_message(&mut self, _: &S, message: &IM) -> Option<event_loop::Handled<OM>> {
        let mut out: Vec<Option<Handled<OM>>> = Vec::new();

        let RawConsoleOutput(console_out): &RawConsoleOutput = message.try_get()?;

        // Check all the single-line stuff
        for line in console_out.lines() {
            // Match status
            if let Some(caps) = self.regex_status.captures(line) {
                match StatusLine::parse(&caps) {
                    Ok(status) => {
                        let status = ConsoleOutput::Status(status);
                        out.push(Handled::single(status));
                    }
                    Err(e) => tracing::error!("Error parsing status line: {:?}", e),
                }
            }
            // Match chat message
            if let Some(caps) = self.regex_chat.captures(line) {
                let chat = ChatMessage::parse(&caps);
                out.push(Handled::single(ConsoleOutput::Chat(chat)));
            }
            // Match player kills
            if let Some(caps) = self.regex_kill.captures(line) {
                let kill = PlayerKill::parse(&caps);
                out.push(Handled::single(ConsoleOutput::Kill(kill)));
            }
            // Match server hostname
            if let Some(caps) = self.regex_hostname.captures(line) {
                let hostname = Hostname::parse(&caps);
                out.push(Handled::single(ConsoleOutput::Hostname(hostname)));
            }
            // Match server IP
            if let Some(caps) = self.regex_ip.captures(line) {
                let ip = ServerIP::parse(&caps);
                out.push(Handled::single(ConsoleOutput::ServerIP(ip)));
            }
            // Match server map
            if let Some(caps) = self.regex_map.captures(line) {
                let map = Map::parse(&caps);
                out.push(Handled::single(ConsoleOutput::Map(map)));
            }
            // Match server player count
            if let Some(caps) = self.regex_playercount.captures(line) {
                let playercount = PlayerCount::parse(&caps);
                out.push(Handled::single(ConsoleOutput::PlayerCount(playercount)));
            }
            // Match demo recording end
            if let Some(caps) = self.regex_demostop.captures(line) {
                let demostop = DemoStop::parse(&caps);
                out.push(Handled::single(ConsoleOutput::DemoStop(demostop)));
            }
        }

        // Check for G15
        let players = self.g15_parser.parse_g15(console_out);
        if !players.is_empty() {
            out.push(Handled::single(ConsoleOutput::G15(players)));
        }

        if out.is_empty() {
            return Handled::none();
        }

        Handled::multiple(out)
    }
}
