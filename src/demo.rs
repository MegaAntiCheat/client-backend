use std::{
    fs::{metadata, File},
    io::{Read, Seek},
    path::{Path, PathBuf},
    sync::mpsc::{self, Sender},
    time::Duration,
};

use bitbuffer::{BitError, BitRead, BitReadBuffer, BitReadStream, LittleEndian};
use event_loop::{try_get, HandlerStruct, Is};
use notify::{event::ModifyKind, Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use steamid_ng::SteamID;
use tf_demo_parser::demo::{
    gameevent_gen::{VoteCastEvent, VoteOptionsEvent},
    gamevent::GameEvent,
    header::Header,
    message::{gameevent::GameEventMessage, Message},
    packet::{message::MessagePacket, Packet},
    parser::{
        gamestateanalyser::{GameState, GameStateAnalyser},
        DemoHandler, RawPacketStream,
    },
};

use crate::state::MACState;

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub struct DemoMessage {
    pub tick: u32,
    pub event: DemoEvent,
}

#[allow(clippy::module_name_repetitions)]
#[derive(Debug, Clone)]
pub enum DemoEvent {
    VoteStarted(Box<VoteOptionsEvent>),
    VoteCast(VoteCastEvent, Option<SteamID>),
    LastestTick,
}

#[allow(clippy::module_name_repetitions)]
pub struct DemoManager {
    previous_demos: Vec<OpenDemo>,
    current_demo: Option<OpenDemo>,

    send: Sender<DemoMessage>,
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
    pub const fn new(send: Sender<DemoMessage>) -> Self {
        Self {
            previous_demos: Vec::new(),
            current_demo: None,
            send,
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
            if let Err(e) = demo.read_next_bytes(&self.send) {
                tracing::error!("Error when reading demo {:?}: {:?}", demo.file_path, e);
                tracing::error!("Demo is being abandoned");
                self.current_demo = None;
            }
        }
    }
}

impl OpenDemo {
    /// Append the provided bytes to the current demo being watched, and handle
    /// any packets
    ///
    /// # Errors
    /// On IO errors, or the demo unexpectedly shortening.
    pub fn read_next_bytes(&mut self, send: &Sender<DemoMessage>) -> std::io::Result<()> {
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
            self.process_next_chunk(send);
        }

        Ok(())
    }

    #[allow(clippy::cognitive_complexity)]
    fn process_next_chunk(&mut self, send: &Sender<DemoMessage>) {
        tracing::debug!("New demo length: {}", self.bytes.len());

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
                    handle_packet(&packet, self.handler.borrow_output(), send);
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

        send.send(DemoMessage {
            tick: self.handler.borrow_output().tick.0,
            event: DemoEvent::LastestTick,
        })
        .expect("Failed to send demo message.");
    }
}

#[allow(clippy::cognitive_complexity)]
fn handle_packet(packet: &Packet, state: &GameState, send: &Sender<DemoMessage>) {
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
                // GameEvent::VoteStarted(e) => {
                //     tracing::info!("Vote started: {:?}", e);
                // }
                GameEvent::VoteOptions(e) => {
                    send.send(DemoMessage {
                        tick: tick.0,
                        event: DemoEvent::VoteStarted(e.clone()),
                    })
                    .expect("Failed to send demo message.");
                }
                GameEvent::VoteCast(e) => {
                    send.send(DemoMessage {
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
                    })
                    .expect("Failed to send demo message.");
                }
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
}

#[allow(clippy::module_name_repetitions)]
/// # Errors
/// If the `Watcher` for file operations could not be created.
///
/// # Panics
/// If the `Watcher` monitoring file changes dies.
pub fn demo_loop(demo_path: &Path, send: Sender<DemoMessage>) -> anyhow::Result<()> {
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

    let mut manager = DemoManager::new(send);
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

pub struct DemoEventWatcher {}

impl<IM, OM> HandlerStruct<MACState, IM, OM> for DemoEventWatcher
where
    IM: Is<DemoMessage>,
{
    fn handle_message(
        &mut self,
        state: &MACState,
        message: &IM,
    ) -> Option<event_loop::Handled<OM>> {
        let DemoMessage { tick, event } = try_get(message)?;

        tracing::info!("{tick}: Got event {event:?}");
        if let DemoEvent::VoteCast(e, Some(steamid)) = event {
            if let Some(gi) = state.players.game_info.get(steamid) {
                tracing::info!("({})", gi.name);
            }
        }

        None
    }
}
