use std::{
    fs::{metadata, File},
    io::{Read, Seek},
    path::{Path, PathBuf},
    sync::mpsc,
    time::Duration,
};

use bitbuffer::{BitError, BitRead, BitReadBuffer, BitReadStream, LittleEndian};
use notify::{event::ModifyKind, Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use tf_demo_parser::demo::{
    gamevent::GameEvent,
    header::Header,
    message::{gameevent::GameEventMessage, Message},
    packet::{message::MessagePacket, Packet},
    parser::{gamestateanalyser::GameStateAnalyser, DemoHandler, RawPacketStream},
};

#[allow(clippy::module_name_repetitions)]
pub struct DemoManager {
    previous_demos: Vec<OpenDemo>,
    current_demo: Option<OpenDemo>,
}

#[allow(clippy::module_name_repetitions)]
pub struct OpenDemo {
    pub file_path: PathBuf,
    pub header: Option<Header>,
    pub handler: DemoHandler<GameStateAnalyser>,
    pub bytes: Vec<u8>,
    pub offset: usize,
}

impl DemoManager {
    /// Create a new `DemoManager`
    #[must_use]
    pub const fn new() -> Self {
        Self {
            previous_demos: Vec::new(),
            current_demo: None,
        }
    }

    /// Start tracking a new demo file. A demo must be being tracked before
    /// bytes can be appended.
    pub fn new_demo(&mut self, path: PathBuf) {
        if let Some(old) = self.current_demo.take() {
            self.previous_demos.push(old);
        }

        // TODO - Change to debug when demo monitoring defaults to on
        tracing::info!("Watching new demo: {:?}", path);

        self.current_demo = Some(OpenDemo {
            file_path: path,
            header: None,
            handler: DemoHandler::with_analyser(GameStateAnalyser::new()),
            bytes: Vec::new(),
            offset: 0,
        });
    }

    pub fn current_demo_path(&self) -> Option<&Path> {
        self.current_demo.as_ref().map(|d| d.file_path.as_path())
    }

    pub fn read_next_bytes(&mut self) {
        if let Some(demo) = self.current_demo.as_mut() {
            if let Err(e) = demo.read_next_bytes() {
                tracing::error!("Error when reading demo {:?}: {:?}", demo.file_path, e);
                tracing::error!("Demo is being abandoned");
                self.current_demo = None;
            }
        }
    }
}

impl Default for DemoManager {
    fn default() -> Self { Self::new() }
}

impl OpenDemo {
    /// Append the provided bytes to the current demo being watched, and handle
    /// any packets
    ///
    /// # Errors
    /// On IO errors, or the demo unexpectedly shortening.
    pub fn read_next_bytes(&mut self) -> std::io::Result<()> {
        let current_metadata = metadata(&self.file_path)?;

        // Check there's actually data to read
        match current_metadata.len().cmp(&(self.bytes.len() as u64)) {
            std::cmp::Ordering::Less => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "Demo has shortened. Something has gone wrong.",
                ));
            }
            std::cmp::Ordering::Equal => {
                return Ok(());
            }
            std::cmp::Ordering::Greater => {}
        }

        let mut file = File::open(&self.file_path)?;
        let last_size = self.bytes.len();

        file.seek(std::io::SeekFrom::Start(last_size as u64))?;
        let read_bytes = file.read_to_end(&mut self.bytes)?;

        if read_bytes > 0 {
            tracing::debug!("Got {} demo bytes", read_bytes);
            self.process_next_chunk();
        }

        Ok(())
    }

    #[allow(clippy::cognitive_complexity)]
    fn process_next_chunk(&mut self) {
        // TODO - Change to debug when demo monitoring defaults to on
        tracing::info!("New demo length: {}", self.bytes.len());

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
                Err(bitbuffer::BitError::NotEnoughData {
                    requested,
                    bits_left,
                }) => {
                    tracing::warn!("Tried to read header but there were not enough bits. Requested: {}, Remaining: {}", requested, bits_left);
                    return;
                }
                Err(e) => {
                    tracing::error!("Error reading demo header: {}", e);
                    return;
                }
            }
        }

        // Parse packets
        let mut packets: RawPacketStream = RawPacketStream::new(stream);
        loop {
            match packets.next(&self.handler.state_handler) {
                Ok(Some(packet)) => {
                    handle_packet(&packet);
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
                    return;
                }
            }
        }
    }
}

#[allow(clippy::cognitive_complexity)]
fn handle_packet(packet: &Packet) {
    if let Packet::Message(MessagePacket {
        tick: _,
        messages,
        meta: _,
    }) = packet
    {
        for m in messages {
            if let Message::GameEvent(GameEventMessage {
                event_type_id: _,
                event,
            }) = m
            {
                match event {
                    GameEvent::VoteStarted(e) => {
                        tracing::info!("Vote started: {:?}", e);
                    }
                    GameEvent::VoteOptions(e) => {
                        tracing::info!("Vote options: {:?}", e);
                    }
                    GameEvent::VoteCast(e) => {
                        tracing::info!("Vote cast: {:?}", e);
                    }
                    GameEvent::VoteEnded(e) => {
                        tracing::info!("Vote ended: {:?}", e);
                    }
                    GameEvent::VotePassed(e) => {
                        tracing::info!("Vote passed: {:?}", e);
                    }
                    GameEvent::VoteFailed(e) => {
                        tracing::info!("Vote failed: {:?}", e);
                    }
                    GameEvent::VoteChanged(e) => {
                        tracing::info!("Vote changed: {:?}", e);
                    }
                    GameEvent::PlayerConnect(e) => {
                        tracing::info!("Player connect: {:?}", e);
                    }
                    GameEvent::PlayerConnectClient(e) => {
                        tracing::info!("Player connect client: {:?}", e);
                    }
                    GameEvent::PlayerInfo(e) => {
                        tracing::info!("Player info: {:?}", e);
                    }
                    GameEvent::Unknown(e) => {
                        tracing::info!("Unknown: {:?}", e);
                    }
                    _ => {}
                }
            }
        }
    }
}

#[allow(clippy::module_name_repetitions)]
/// # Errors
/// If the `Watcher` for file operations could not be created.
///
/// # Panics
/// If the `Watcher` monitoring file changes dies.
pub fn demo_loop(demo_path: &Path) -> anyhow::Result<()> {
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

    // Create a tick interval to periodically check metadata
    let metadata_tick = Duration::from_secs(5);

    tracing::debug!("Demo loop started");

    let mut manager = DemoManager::new();
    loop {
        match rx.recv_timeout(metadata_tick) {
            Ok(event) => {
                let path = &event.paths[0];
                match event.kind {
                    notify::event::EventKind::Create(_) => {
                        if path.extension().map_or(false, |ext| ext == "dem") {
                            manager.new_demo(path.clone());
                        }
                    }
                    notify::event::EventKind::Modify(ModifyKind::Data(_)) => {
                        if manager.current_demo_path().is_some_and(|p| p == path) {
                            manager.read_next_bytes();
                        } else if path.extension().map_or(false, |ext| ext == "dem") {
                            // A new demo can be started with the same name as a previous one, or
                            // the player can be already connected to a
                            // server and recording a demo when the application is run.
                            // This should catch those cases.
                            manager.new_demo(path.clone());
                            manager.read_next_bytes();
                        }
                    }
                    _ => {}
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {
                manager.read_next_bytes();
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                panic!("Couldn't receive demo updates. Watcher died.");
            }
        }
    }
}
