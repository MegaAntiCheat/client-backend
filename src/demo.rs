use std::{
    fs::{metadata, File},
    io::{Read, Seek},
    path::{Path, PathBuf},
    sync::mpsc::{self, Receiver, TryRecvError},
    time::{Duration, Instant},
};

use bitbuffer::{BitError, BitRead, BitReadBuffer, BitReadStream, LittleEndian};
use event_loop::{try_get, Handled, HandlerStruct, Is, MessageSource};
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
use thiserror::Error;

use crate::masterbase::DemoSession;

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
#[derive(Debug, Error)]
pub enum DemoWatcherError {
    #[error("Notify error: {0}")]
    Notify(#[from] notify::Error),
}

#[allow(clippy::module_name_repetitions)]
pub struct DemoBytes {
    pub path: PathBuf,
    pub bytes: Vec<u8>,
}

#[allow(clippy::module_name_repetitions)]
pub struct DemoWatcher {
    recv: Receiver<Event>,
    last_recv: Instant,
    disconnected: bool,

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
                return Err(std::io::Error::new(
                    std::io::ErrorKind::UnexpectedEof,
                    "Demo has shortened. Something has gone wrong.",
                ));
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
        let Some(file_path) = self.current_demo.clone() else {
            return None;
        };

        self.read_next_bytes()
            .map_err(|e| tracing::error!("Failed reading bytes from demo {file_path:?}: {e}"))
            .ok()
            .flatten()
            .map(|b| DemoBytes {
                path: file_path,
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

#[allow(clippy::module_name_repetitions)]
pub struct DemoManager {
    previous_demos: Vec<OpenDemo>,
    current_demo: Option<OpenDemo>,

    upload_demos: bool,
    session: Option<DemoSession>,
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
    pub const fn new(upload_demos: bool) -> Self {
        Self {
            previous_demos: Vec::new(),
            current_demo: None,

            upload_demos,
            session: None,
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
}

impl<S, IM, OM> HandlerStruct<S, IM, OM> for DemoManager
where
    IM: Is<DemoBytes>,
    OM: Is<DemoMessage>,
{
    fn handle_message(&mut self, _state: &S, message: &IM) -> Option<event_loop::Handled<OM>> {
        let msg = try_get(message)?;

        tracing::info!("Got {} bytes for demo {:?}", msg.bytes.len(), msg.path);

        // TODO - Open sessions and stuff

        // New or different demo
        if self.current_demo.is_none()
            || self
                .current_demo
                .as_ref()
                .map(|d| d.file_path != msg.path)
                .is_some_and(|b| b)
        {
            self.new_demo(msg.path.clone());
        }

        Handled::multiple(
            self.current_demo
                .as_mut()
                .expect("self.new_demo should have guaranteed a valid demo is present")
                .append_bytes(&msg.bytes)
                .into_iter()
                .map(Handled::single),
        )
    }
}

impl OpenDemo {
    /// Append the provided bytes to the current demo being watched, and handle
    /// any packets
    pub fn append_bytes(&mut self, bytes: &[u8]) -> Vec<DemoMessage> {
        if bytes.is_empty() {
            return Vec::new();
        }

        self.bytes.extend_from_slice(bytes);
        self.process_next_chunk()
    }

    /// Attempt to parse any new bytes that have been added since the last call as packets
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
                Err(bitbuffer::BitError::NotEnoughData {
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
                // GameEvent::VoteStarted(e) => {
                //     tracing::info!("Vote started: {:?}", e);
                // }
                GameEvent::VoteOptions(e) => out.push(DemoMessage {
                    tick: tick.0,
                    event: DemoEvent::VoteStarted(e.clone()),
                }),
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
