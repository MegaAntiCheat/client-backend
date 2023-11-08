use anyhow::{anyhow, Context, Result};
use bitbuffer::{BitError, BitRead, BitReadBuffer, BitReadStream, LittleEndian};
use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tf_demo_parser::demo::header::Header;
use tf_demo_parser::demo::parser::{DemoHandler, NullHandler, RawPacketStream};

use tokio::fs::{metadata, File};
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::sync::mpsc;
use tokio::time::{interval, Duration};
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

pub struct DemoManager<'a> {
    previous_demos: Vec<OpenDemo<'a>>,
    current_demo: Option<OpenDemo<'a>>,
}

pub struct OpenDemo<'a> {
    pub file_path: PathBuf,
    pub header: Option<Header>,
    pub handler: DemoHandler<'a, NullHandler>,
    pub bytes: Vec<u8>,
    pub offset: usize,
}

impl<'a> DemoManager<'a> {
    /// Create a new DemoManager
    pub fn new() -> DemoManager<'a> {
        DemoManager {
            previous_demos: Vec::new(),
            current_demo: None,
        }
    }

    /// Start tracking a new demo file. A demo must be being tracked before bytes can be appended.
    pub fn new_demo(&mut self, path: PathBuf) {
        if let Some(old) = self.current_demo.take() {
            self.previous_demos.push(old);
        }

        tracing::debug!("Watching new demo: {:?}", path);

        self.current_demo = Some(OpenDemo {
            file_path: path,
            header: None,
            handler: DemoHandler::default(),
            bytes: Vec::new(),
            offset: 0,
        });
    }

    pub fn current_demo_path(&self) -> Option<&Path> {
        self.current_demo.as_ref().map(|d| d.file_path.as_path())
    }

    pub async fn read_next_bytes(&mut self) {
        if let Some(demo) = self.current_demo.as_mut() {
            if let Err(e) = demo.read_next_bytes().await {
                tracing::error!("Error when reading demo {:?}: {:?}", demo.file_path, e);
                tracing::error!("Demo is being abandoned");
                self.current_demo = None;
            }
        }
    }
}

impl<'a> OpenDemo<'a> {
    /// Append the provided bytes to the current demo being watched, and handle any packets
    pub async fn read_next_bytes(&mut self) -> Result<()> {
        let current_metadata = metadata(&self.file_path)
            .await
            .context("Couldn't read demo metadata")?;

        // Check there's actually data to read
        if current_metadata.len() < self.bytes.len() as u64 {
            return Err(anyhow!("Demo has shortened. Something has gone wrong."));
        } else if current_metadata.len() == self.bytes.len() as u64 {
            return Ok(());
        }

        let mut file = File::open(&self.file_path).await?;
        let last_size = self.bytes.len();

        file.seek(SeekFrom::Start(last_size as u64)).await?;
        let read_bytes = file.read_to_end(&mut self.bytes).await?;

        if read_bytes > 0 {
            tracing::debug!("Got {} demo bytes", read_bytes);
            self.process_next_chunk().await;
        }

        Ok(())
    }

    async fn process_next_chunk(&mut self) {
        tracing::debug!("New demo length: {}", self.bytes.len());

        let buffer = BitReadBuffer::new(&self.bytes, LittleEndian);
        let mut stream = BitReadStream::new(buffer);
        stream.set_pos(self.offset).unwrap();

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

        let mut packets: RawPacketStream = RawPacketStream::new(stream);
        match packets.next(&self.handler.state_handler) {
            Ok(Some(packet)) => {
                tracing::info!("Packet: {:?}", packet);
                self.handler.handle_packet(packet).unwrap();
                self.offset = packets.pos();
            }
            Ok(None) => {
                tracing::info!("No packet");
            }
            Err(tf_demo_parser::ParseError::ReadError(BitError::NotEnoughData {
                requested,
                bits_left,
            })) => {
                tracing::warn!("Tried to read header but there were not enough bits. Requested: {}, Remaining: {}", requested, bits_left);
                return;
            }
            Err(e) => {
                tracing::error!("Error reading demo packet: {}", e);
            }
        }
    }
}

#[derive(Serialize, Deserialize, Debug)]
pub struct BigBrotherPacket {
    players: Vec<String>,  // List of players on the server, array of SteamIDs
    ip: String,            // IP address of the server
    sequence: i32, // A sequence number that starts at 0 and increments for each successive fragment
    demo: Option<Vec<u8>>, // Up-to-date copy of demo file. None for packet 0 as the demo wouldn't be flushed to disk yet.
}

pub async fn demo_loop(demo_path: PathBuf) -> anyhow::Result<()> {
    let (tx, rx) = mpsc::unbounded_channel();
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

    watcher.watch(demo_path.as_path(), RecursiveMode::NonRecursive)?;

    let mut rx = UnboundedReceiverStream::new(rx);

    // Create a tick interval to periodically check metadata
    let mut metadata_tick = interval(Duration::from_secs(5));

    tracing::info!("Demo loop started");

    let mut manager = DemoManager::new();

    loop {
        tokio::select! {
            // Handle file events
            Some(event) = rx.next() => {
                let path = &event.paths[0];
                match event.kind {
                    notify::event::EventKind::Create(_) => {
                        if path.extension().map_or(false, |ext| ext == "dem") {
                            manager.new_demo(path.clone());
                        }
                    }
                    notify::event::EventKind::Modify(_) => {
                        if manager.current_demo_path().map(|p| p == path).unwrap_or(false) {
                            manager.read_next_bytes().await;
                        }
                    }
                    _ => {
                        tracing::debug!("Unhandled event kind: {:?}", event.kind);
                    }
                }
            },
            // Handle metadata tick
            _ = metadata_tick.tick() => {
                // If there is a current file being watched, check its metadata
                manager.read_next_bytes().await;
            },
        }
    }
}
