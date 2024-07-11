use bitbuffer::{BitError, BitRead, BitReadBuffer, BitReadStream, LittleEndian};
use event_loop::{try_get, Handled, Is, MessageHandler, MessageSource};
use notify::{event::ModifyKind, Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::{
    collections::HashMap,
    fs::{metadata, File},
    io::{Read, Seek},
    path::{Path, PathBuf},
    sync::{
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    time::{Duration, Instant},
};
use steamid_ng::SteamID;
use tf_demo_parser::demo::{
    gameevent_gen::{VoteCastEvent, VoteOptionsEvent, VoteStartedEvent},
    gamevent::GameEvent,
    header::Header,
    message::{gameevent::GameEventMessage, Message},
    packet::{message::MessagePacket, Packet},
    parser::{
        gamestateanalyser::{GameState, GameStateAnalyser},
        DemoHandler, RawPacketStream,
    },
};
use thiserror::Error;
use tokio::sync::{Mutex, MutexGuard};

use crate::{
    events::UserUpdates,
    masterbase::{DemoSession, ReportReason},
    new_players::NewPlayers,
    settings::Settings,
    state::MACState,
};

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub struct DemoMessage {
    pub tick: u32,
    pub event: DemoEvent,
}
impl<S> event_loop::Message<S> for DemoMessage {}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub enum DemoEvent {
    VoteStarted(VoteStartedEvent),
    VoteOptions(Box<VoteOptionsEvent>),
    VoteCast(VoteCastEvent, Option<SteamID>),
    LatestTick,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Error)]
pub enum DemoWatcherError {
    #[error("Notify error: {0}")]
    Notify(#[from] notify::Error),
}

#[allow(clippy::module_name_repetitions)]
pub struct DemoBytes {
    pub file_path: PathBuf,
    pub id: usize,
    pub bytes: Vec<u8>,
}
impl<S> event_loop::Message<S> for DemoBytes {}

#[allow(clippy::module_name_repetitions)]
pub struct DemoWatcher {
    recv: Receiver<Event>,
    last_recv: Instant,
    disconnected: bool,

    current_id: usize,
    current_demo: Option<PathBuf>,
    offset: u64,

    _watcher: RecommendedWatcher,
}

impl DemoWatcher {
    /// # Errors
    /// If the [`notify::Watcher`] could not be started.
    pub fn new(demo_path: &Path) -> Result<Self, DemoWatcherError> {
        let (tx, rx) = mpsc::channel();
        let config = Config::default().with_poll_interval(Duration::from_secs(2));

        let mut watcher: RecommendedWatcher = Watcher::new(
            Box::new(move |res: Result<Event, notify::Error>| match res {
                Ok(event) => {
                    let _ = tx.send(event);
                }
                Err(err) => {
                    tracing::error!("Error while watching for file changes: {}", err);
                }
            }),
            config,
        )?;

        watcher.watch(demo_path, RecursiveMode::Recursive)?;

        Ok(Self {
            recv: rx,
            last_recv: Instant::now(),
            disconnected: false,
            current_id: 0,
            current_demo: None,
            offset: 0,
            _watcher: watcher,
        })
    }

    /// Return the next chunk of bytes for the current demo being watched
    ///
    /// # Errors
    /// On IO errors, or the demo unexpectedly shortening.
    fn read_next_bytes(&mut self) -> std::io::Result<Option<Vec<u8>>> {
        let Some(file_path) = self.current_demo.as_ref() else {
            return Ok(None);
        };

        let current_metadata = metadata(file_path)?;

        // Check there's actually data to read
        match current_metadata.len().cmp(&(self.offset)) {
            std::cmp::Ordering::Less => {
                tracing::warn!("Demo has shortened. Assuming a new demo has been recorded over the existing one.");
                self.current_id += 1;
                self.offset = 0;
            }
            std::cmp::Ordering::Equal => {
                return Ok(None);
            }
            std::cmp::Ordering::Greater => {}
        }

        let mut file = File::open(file_path)?;
        let last_size = self.offset;

        file.seek(std::io::SeekFrom::Start(last_size))?;
        let mut out = Vec::new();
        let read_bytes = file.read_to_end(&mut out)?;

        if read_bytes > 0 {
            tracing::debug!("Got {} demo bytes", read_bytes);
            self.offset += read_bytes as u64;
        }

        Ok(Some(out))
    }

    fn next_bytes(&mut self) -> Option<DemoBytes> {
        let file_path = self.current_demo.clone()?;

        self.read_next_bytes()
            .map_err(|e| tracing::error!("Failed reading bytes from demo {file_path:?}: {e}"))
            .ok()
            .flatten()
            .map(|b| DemoBytes {
                id: self.current_id,
                file_path,
                bytes: b,
            })
    }
}

impl<M: Is<DemoBytes>> MessageSource<M> for DemoWatcher {
    fn next_message(&mut self) -> Option<M> {
        match self.recv.try_recv() {
            Ok(e) => {
                let path = &e.paths[0];
                match e.kind {
                    notify::event::EventKind::Create(_) => {
                        if path.extension().map_or(false, |ext| ext == "dem") {
                            self.current_demo = Some(path.clone());
                            self.offset = 0;
                        }
                        return self.next_bytes().map(Into::into);
                    }
                    notify::event::EventKind::Modify(ModifyKind::Data(_)) => {
                        if self.current_demo.as_ref().is_some_and(|p| p == path) {
                            return self.next_bytes().map(Into::into);
                        } else if path.extension().map_or(false, |ext| ext == "dem") {
                            // A new demo can be started with the same name as a previous one, or
                            // the player can be already connected to a
                            // server and recording a demo when the application is run.
                            // This should catch those cases.
                            self.current_demo = Some(path.clone());
                            self.offset = 0;
                            return self.next_bytes().map(Into::into);
                        }
                    }
                    _ => {}
                }
            }
            Err(TryRecvError::Disconnected) => {
                if self.disconnected {
                    return None;
                }

                tracing::error!("Lost connection to demo watcher...");
                self.disconnected = true;
            }
            _ => {}
        }

        if self.last_recv.elapsed().as_secs() > 3 {
            self.last_recv = Instant::now();
            return self.next_bytes().map(Into::into);
        }

        None
    }
}

enum SessionMissingReason {
    Uninit,
    Disabled,
    Error,
    Closed,
}

#[allow(clippy::module_name_repetitions)]
pub struct DemoManager {
    previous_demos: Vec<OpenDemo>,
    current_demo: Option<OpenDemo>,

    session: DemoManagerSession,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Clone)]
struct DemoManagerSession(Arc<Mutex<Result<DemoSession, SessionMissingReason>>>);

#[allow(clippy::module_name_repetitions)]
pub struct OpenDemo {
    pub file_path: PathBuf,
    pub id: usize,
    pub header: Option<Header>,
    pub handler: DemoHandler<GameStateAnalyser>,
    pub bytes: Vec<u8>,
    pub offset: usize,
}

impl PartialEq for OpenDemo {
    fn eq(&self, other: &Self) -> bool {
        self.file_path == other.file_path && self.id == other.id
    }
}

impl DemoManagerSession {
    pub fn new(missing: SessionMissingReason) -> Self {
        Self(Arc::new(Mutex::new(Err(missing))))
    }

    /// Waits until the session is initialised, then returns a lock for it
    pub async fn get(&mut self) -> MutexGuard<Result<DemoSession, SessionMissingReason>> {
        loop {
            let guard: MutexGuard<_> = self.0.lock().await;

            if matches!(*guard, Err(SessionMissingReason::Uninit)) {
                drop(guard);
                tokio::time::sleep(Duration::from_millis(5)).await;
                continue;
            }

            return guard;
        }
    }

    /// Returns an event which opens a new session.
    /// This event needs to be handled by the event loop to take effect.
    fn open_new_session<M: Is<DemoMessage>>(
        &mut self,
        settings: &Settings,
        header: &Header,
        demo_name: &str,
    ) -> Option<Handled<M>> {
        let host = settings.masterbase_host().to_owned();
        let key = settings.masterbase_key().to_owned();
        let map = header.map.clone();
        let fake_ip = header.server.clone();
        let http = settings.use_masterbase_http();
        let demo_name = demo_name.to_owned();
        let session = self.0.clone();

        Handled::future(async move {
            let session = session;
            let mut maybe_session = session.lock().await;
            assert!(maybe_session.is_err());

            // Create session
            match DemoSession::new(host, key, &fake_ip, &map, &demo_name, http).await {
                Ok(session) => {
                    tracing::info!("Opened new demo session with Masterbase: {session:?}");
                    *maybe_session = Ok(session);
                }
                Err(e) => {
                    tracing::error!("Could not open new demo session: {e}");
                    *maybe_session = Err(SessionMissingReason::Error);
                }
            }

            None
        })
    }
}

impl DemoManager {
    /// Create a new `DemoManager`
    #[must_use]
    pub fn new() -> Self {
        Self {
            previous_demos: Vec::new(),
            current_demo: None,

            session: DemoManagerSession::new(SessionMissingReason::Disabled),
        }
    }

    /// Start tracking a new demo file. A demo must be being tracked before
    /// bytes can be appended.
    fn new_demo(&mut self, path: PathBuf, id: usize, uploads_enabled: bool) {
        if let Some(old) = self.current_demo.take() {
            self.previous_demos.push(old);
        }

        tracing::debug!("Watching new demo: {:?}", path);

        self.current_demo = Some(OpenDemo {
            file_path: path,
            id,
            header: None,
            handler: DemoHandler::with_analyser(GameStateAnalyser::new()),
            bytes: Vec::new(),
            offset: 0,
        });

        self.session = DemoManagerSession::new(if uploads_enabled {
            SessionMissingReason::Uninit
        } else {
            SessionMissingReason::Disabled
        });
    }

    fn current_demo_path(&self) -> Option<&Path> {
        self.current_demo.as_ref().map(|d| d.file_path.as_path())
    }

    /// Reads the bytes that are only written at the end of the demo recording.
    ///
    /// # Errors
    /// On IO errors
    fn read_late_bytes(&self) -> std::io::Result<Option<Vec<u8>>> {
        let Some(file_path) = self.current_demo_path() else {
            return Ok(None);
        };

        let start_address = 0x420;
        let bytes_to_read: usize = 16;
        let min_valid_filelen: u64 = start_address + bytes_to_read as u64;

        let current_metadata = metadata(file_path)?;

        // Check the file is long enough to have data at the late byte address
        match current_metadata.len().cmp(&min_valid_filelen) {
            std::cmp::Ordering::Less => {
                return Ok(None);
            }
            std::cmp::Ordering::Equal | std::cmp::Ordering::Greater => {}
        }

        let mut file = File::open(file_path)?;

        file.seek(std::io::SeekFrom::Start(start_address))?;
        let mut out = vec![0; bytes_to_read];
        file.read_exact(&mut out)?;

        // Check if the late bytes have been written to
        // The first 8 bytes are always all zeroes until written to.
        let written = out.len() > 8 && out.iter().take(8).any(|&b| b != 0);

        if written {
            tracing::debug!("Late bytes found in demo recording.");
            Ok(Some(out))
        } else {
            tracing::debug!("No new bytes from demo.");
            Ok(None)
        }
    }

    /// Returns an event that uploads the given bytes to the current session.
    /// This event needs to be handled by the event loop to take effect.
    fn upload_bytes<M: Is<DemoMessage>>(&mut self, bytes: Vec<u8>) -> Option<Handled<M>> {
        // Loop while session is uninit
        let mut session = self.session.clone();
        Handled::future(async move {
            let mut guard = session.get().await;
            match &mut *guard {
                Ok(session) => {
                    let len = bytes.len();
                    if let Err(e) = session.send_bytes(bytes).await {
                        tracing::error!("Failed to upload demo chunk: {e}");
                        *guard = Err(SessionMissingReason::Error);
                        drop(guard);
                    } else {
                        tracing::debug!("Uploaded {len} bytes to masterbase.");
                    }
                }
                Err(SessionMissingReason::Closed) => {
                    tracing::error!("Tried to upload bytes after demo session was closed.");
                }
                Err(
                    SessionMissingReason::Error
                    | SessionMissingReason::Disabled
                    | SessionMissingReason::Uninit,
                ) => {}
            }
            None
        })
    }

    /// Returns an event that checks for and handles the late bytes for the
    /// current demo.
    /// This event needs to be handled by the event loop to take effect.
    fn handle_late_bytes<M: Is<DemoMessage>>(&self, late_bytes: Vec<u8>) -> Option<Handled<M>> {
        let mut session = self.session.clone();
        Handled::future(async move {
            let mut session_lock = session.get().await;
            let Ok(session) = &mut *session_lock else {
                // Drop session
                *session_lock = Err(SessionMissingReason::Closed);
                return None;
            };

            match session.send_late_bytes(late_bytes).await {
                Ok(send_response) => {
                    let status = send_response.status();
                    if status.is_success() {
                        tracing::debug!(
                            "Uploaded late bytes to masterbase. Attempting to close session..."
                        );
                    } else {
                        let s = status.as_str();
                        tracing::error!(
                            "Failed to upload late bytes to masterbase: Server returned {s}"
                        );
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to upload late bytes to masterbase: {e}");
                }
            }

            // Drop session
            *session_lock = Err(SessionMissingReason::Closed);
            None
        })
    }

    /// Reports any other the players provided who are marked as bots to the masterbase
    fn report_players<M>(
        &mut self,
        players: impl Iterator<Item = (SteamID, ReportReason)>,
    ) -> Option<Handled<M>> {
        Handled::multiple(players.map(|(s, r)| {
            let mut session = self.session.clone();
            Handled::future(async move {
                let mut session_guard = session.get().await;
                let Ok(session) = &mut *session_guard else {
                    return None;
                };

                let resp = session.report_player(s, r).await;
                drop(session_guard);

                match resp {
                    Ok(resp) if resp.status().is_success() => {
                        tracing::info!("Reported {} as {r:?}", u64::from(s));
                    }
                    Ok(resp) => {
                        tracing::error!(
                            "Failed to report account {} as {r:?}: {}",
                            u64::from(s),
                            resp.status()
                        );
                    }
                    Err(e) => {
                        tracing::error!(
                            "Failed to report account {} as {r:?}: {}",
                            u64::from(s),
                            e
                        );
                    }
                }

                None
            })
        }))
    }

    fn handle_demo_bytes<M: Is<DemoMessage>>(
        &mut self,
        state: &MACState,
        msg: &DemoBytes,
    ) -> Option<Handled<M>> {
        tracing::debug!("Got {} bytes for demo {:?}", msg.bytes.len(), msg.file_path);

        // New or different demo
        if self
            .current_demo
            .as_ref()
            .map_or(true, |d| !(d.file_path == msg.file_path && d.id == msg.id))
        {
            self.new_demo(msg.file_path.clone(), msg.id, state.settings.upload_demos);
        }

        let demo = self
            .current_demo
            .as_mut()
            .expect("self.new_demo should have guaranteed a valid demo is present.");

        let parsed_header = demo.header.is_some();

        let mut events = Vec::new();

        // Don't parse contents if the user only wants minimal parsing, except
        // if we still need to extract the headers.
        if !(parsed_header && state.settings.minimal_demo_parsing()) {
            events.extend(
                demo.append_bytes(&msg.bytes)
                    .into_iter()
                    .map(Handled::single),
            );
        }

        if !state.settings.upload_demos() {
            return Handled::multiple(events);
        }

        // let file_name: &str = msg.file_path.file_name().unwrap().to_str().unwrap();
        let file_name = msg
            .file_path
            .file_name()
            .expect("Should be valid file.")
            .to_string_lossy();

        // Open new demo session if we've extracted the header
        if let Some(header) = demo.header.as_ref() {
            if !parsed_header {
                events.push(
                    self.session
                        .open_new_session(&state.settings, header, &file_name),
                );

                // Once a new session is opened, report any bots already on the server
                events.push(
                    self.report_players(
                        // Go from SteamID to (SteamID, ReportReason) if the player is marked as a cheater or bot
                        state
                            .players
                            .connected
                            .iter()
                            .map(|&p| (p, state.players.verdict(p)))
                            .filter_map(|(s, r)| ReportReason::try_from(r).ok().map(|r| (s, r))),
                    ),
                );
            }
        }

        // Upload bytes
        let bytes = msg.bytes.clone();
        events.push(self.upload_bytes(bytes));

        // Check for late bytes
        if let Ok(Some(late_bytes)) = self.read_late_bytes() {
            events.push(self.handle_late_bytes(late_bytes));
        }

        Handled::multiple(events)
    }
}

impl Default for DemoManager {
    fn default() -> Self {
        Self::new()
    }
}

impl<IM, OM> MessageHandler<MACState, IM, OM> for DemoManager
where
    IM: Is<DemoBytes> + Is<NewPlayers> + Is<UserUpdates>,
    OM: Is<DemoMessage>,
{
    fn handle_message(&mut self, state: &MACState, message: &IM) -> Option<Handled<OM>> {
        // Report newly connecting bots
        if let Some(players) = try_get::<NewPlayers>(message) {
            return self.report_players(
                players
                    .0
                    .iter()
                    .map(|&p| (p, state.players.verdict(p)))
                    .filter_map(|(s, r)| ReportReason::try_from(r).ok().map(|r| (s, r))),
            );
        }

        // Report newly marked bots
        if let Some(updates) = try_get::<UserUpdates>(message) {
            return self.report_players(
                updates
                    .0
                    .iter()
                    .filter_map(|(&s, u)| u.local_verdict.map(|v| (s, v)))
                    .filter_map(|(s, v)| ReportReason::try_from(v).ok().map(|r| (s, r))),
            );
        }

        // Demo bytes
        if let Some(demo_bytes) = try_get::<DemoBytes>(message) {
            return self.handle_demo_bytes(state, demo_bytes);
        }

        None
    }
}

impl OpenDemo {
    /// Append the provided bytes to the current demo being watched, and handle
    /// any packets
    fn append_bytes(&mut self, bytes: &[u8]) -> Vec<DemoMessage> {
        if bytes.is_empty() {
            return Vec::new();
        }

        self.bytes.extend_from_slice(bytes);
        self.process_next_chunk()
    }

    /// Attempt to parse any new bytes that have been added since the last call
    /// as packets
    #[allow(clippy::cognitive_complexity)]
    fn process_next_chunk(&mut self) -> Vec<DemoMessage> {
        tracing::debug!("New demo length: {}", self.bytes.len());

        let mut out = Vec::new();

        let buffer = BitReadBuffer::new(&self.bytes, LittleEndian);
        let mut stream = BitReadStream::new(buffer);
        stream
            .set_pos(self.offset)
            .expect("Couldn't set stream position");

        // Parse header if there isn't one already
        if self.header.is_none() {
            match Header::read(&mut stream) {
                Ok(header) => {
                    self.handler.handle_header(&header);
                    self.header = Some(header);
                    self.offset = stream.pos();
                }
                Err(BitError::NotEnoughData {
                    requested,
                    bits_left,
                }) => {
                    tracing::warn!("Tried to read header but there were not enough bits. Requested: {}, Remaining: {}", requested, bits_left);
                    return out;
                }
                Err(e) => {
                    tracing::error!("Error reading demo header: {}", e);
                    return out;
                }
            }
        }

        // Parse packets
        let mut packets: RawPacketStream = RawPacketStream::new(stream);
        loop {
            match packets.next(&self.handler.state_handler) {
                Ok(Some(packet)) => {
                    out.append(&mut handle_packet(&packet, self.handler.borrow_output()));
                    self.handler
                        .handle_packet(packet)
                        .expect("Couldn't handle packet");
                    self.offset = packets.pos();
                }
                Ok(None) => {
                    break;
                }
                Err(tf_demo_parser::ParseError::ReadError(BitError::NotEnoughData {
                    requested,
                    bits_left,
                })) => {
                    tracing::warn!("Tried to read packet but there were not enough bits. Requested: {}, Remaining: {}", requested, bits_left);
                    break;
                }
                Err(e) => {
                    tracing::error!("Error reading demo packet: {}", e);
                    return out;
                }
            }
        }

        out
    }
}

#[allow(clippy::cognitive_complexity)]
fn handle_packet(packet: &Packet, state: &GameState) -> Vec<DemoMessage> {
    let mut out = Vec::new();

    if let Packet::Message(MessagePacket {
        tick,
        messages,
        meta: _,
    }) = packet
    {
        for m in messages {
            let Message::GameEvent(GameEventMessage {
                event_type_id: _,
                event_type: _,
                event,
            }) = m
            else {
                continue;
            };

            match event {
                // This event exists as per the Source Demo spec, but has never been extracted by tf-demo-parser
                // We should not rely on it ever occuring, but ideally it should. This may 'just work (tm)' in
                // the future after a dependency bump.
                GameEvent::VoteStarted(e) => {
                    // output a message if we ever do actually see this event, because we should break out
                    // the wine and celebrate
                    tracing::info!("New vote started -> {e:?}");
                    out.push(DemoMessage {
                        tick: tick.0,
                        event: DemoEvent::VoteStarted(e.clone()),
                    });
                }
                // This is actually the first vote event we should see, since we don't see VoteStarted events.
                // If the options are Yes/No (and not map/mode selects), then its MOST LIKELY a votekick. The
                // next immediate VoteCast options should indicate the caller and the target.
                GameEvent::VoteOptions(e) => out.push(DemoMessage {
                    tick: tick.0,
                    event: DemoEvent::VoteOptions(e.clone()),
                }),
                // Simply indicates the player who voted, and what VoteOption they selected, and on what VoteIdx
                GameEvent::VoteCast(e) => out.push(DemoMessage {
                    tick: tick.0,
                    event: DemoEvent::VoteCast(
                        e.clone(),
                        state.players.iter().find_map(|p| {
                            p.info.as_ref().and_then(|i| {
                                if i.entity_id == e.entity_id {
                                    SteamID::from_steam3(&i.steam_id).ok()
                                } else {
                                    None
                                }
                            })
                        }),
                    ),
                }),
                // GameEvent::VoteEnded(e) => {
                //     tracing::info!("Vote ended: {:?}", e);
                // }
                // GameEvent::VotePassed(e) => {
                //     tracing::info!("Vote passed: {:?}", e);
                // }
                // GameEvent::VoteFailed(e) => {
                //     tracing::info!("Vote failed: {:?}", e);
                // }
                // GameEvent::VoteChanged(e) => {
                //     tracing::info!("Vote changed: {:?}", e);
                // }
                // GameEvent::PlayerConnect(e) => {
                //     tracing::info!("Player connect: {:?}", e);
                // }
                // GameEvent::PlayerConnectClient(e) => {
                //     tracing::info!("Player connect client: {:?}", e);
                // }
                // GameEvent::PlayerInfo(e) => {
                //     tracing::info!("Player info: {:?}", e);
                // }
                // GameEvent::Unknown(e) => {
                //     tracing::info!("Unknown: {:?}", e);
                // }
                _ => {}
            }
        }
    }

    out
}

pub struct PrintVotes {
    votes: HashMap<u32, Vec<String>>,
    shunted_vote_cast_messages: Vec<(VoteCastEvent, Option<SteamID>)>,
}

impl PrintVotes {
    #[must_use]
    pub fn new() -> Self {
        Self {
            votes: HashMap::new(),
            shunted_vote_cast_messages: vec![],
        }
    }

    /// Given a `VoteCastEvent`, Optional `SteamID` and the current `MACState`, either return a vote cast string
    /// ('vote option' - 'alias associated with steamID'), or `None` if there is no recognised vote index currently
    /// stored
    fn get_vote_cast_event_message(
        &self,
        event: &VoteCastEvent,
        steamid: &Option<SteamID>,
        state: &MACState,
    ) -> Option<String> {
        let name = steamid
            .as_ref()
            .and_then(|&id| state.players.get_name(id))
            .unwrap_or("Someone");

        let vote = self
            .votes
            .get(&event.voteidx)
            .and_then(|v| v.get(event.vote_option as usize));

        vote.map(|matched_vote| format!("{matched_vote} - {name}"))
    }
}

impl Default for PrintVotes {
    fn default() -> Self {
        Self::new()
    }
}

impl<IM, OM> MessageHandler<MACState, IM, OM> for PrintVotes
where
    IM: Is<DemoMessage>,
{
    #[allow(clippy::cognitive_complexity)]
    fn handle_message(&mut self, state: &MACState, message: &IM) -> Option<Handled<OM>> {
        let msg = try_get(message)?;

        match &msg.event {
            DemoEvent::VoteOptions(options) => 'voteOptionsEvent: {
                let mut values = Vec::new();
                tracing::info!("Vote options:");
                for i in 0..options.count {
                    let opt = match i {
                        0 => options.option_1.to_string(),
                        1 => options.option_2.to_string(),
                        2 => options.option_3.to_string(),
                        3 => options.option_4.to_string(),
                        4 => options.option_5.to_string(),
                        _ => String::new(),
                    };

                    tracing::info!("\t{}", opt);
                    values.push(opt);
                }
                self.votes.insert(options.voteidx, values);

                // Replay shunted messages if we have them. This ensures that we don't print VoteCast events for Vote we haven't seen the
                // VoteOptions event for yet. Saves
                if self.shunted_vote_cast_messages.is_empty() {
                    break 'voteOptionsEvent;
                }

                // We need to temporarily move the event queue into a local buffer so we can immutably borrow self
                // inside the closure. Once we are done, we move the queue back into self.shunted_vote_cast_messages
                let mut temp = Vec::new();
                std::mem::swap(&mut temp, &mut self.shunted_vote_cast_messages);
                temp.retain(|(event, steamid)| {
                    // If we have a shunted message for this voteidx (because we saw the vote cast event before the vote options event)
                    // Then retrieve it and print it now.
                    if let Some(event_str) = self.get_vote_cast_event_message(event, steamid, state)
                    {
                        tracing::debug!("Recalled a shunted VoteCastEvent message.");
                        tracing::info!("{event_str}");
                        return false;
                    }
                    true
                });
                std::mem::swap(&mut temp, &mut self.shunted_vote_cast_messages);
            }
            DemoEvent::VoteCast(event, steamid) => {
                let resp = self.get_vote_cast_event_message(event, steamid, state);
                // If we get a None back, it means we don't have a vote idx stored yet for the vote
                // this is cast on. I.e. we haven't processed a VoteOptions event yet. So we shunt
                // these messages until we do.
                if let Some(output) = resp {
                    tracing::info!("{}", output);
                } else {
                    tracing::debug!(
                        "Shunted a VoteCast message. Total in limbo: {}",
                        self.shunted_vote_cast_messages.len()
                    );
                    self.shunted_vote_cast_messages
                        .push((event.clone(), *steamid));
                }
            }
            DemoEvent::VoteStarted(event) => {
                let issue = event.issue.as_ref();
                let initiator = event.initiator;
                tracing::info!("{issue} - called by {initiator}");
            }
            DemoEvent::LatestTick => {}
        }

        None
    }
}
