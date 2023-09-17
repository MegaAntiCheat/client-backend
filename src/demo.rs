use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use tokio::fs::metadata;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio::time::Duration;
use tokio_stream::wrappers::UnboundedReceiverStream;
use tokio_stream::StreamExt;

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

    tracing::debug!("Demo loop started");

    let mut current_file_path = PathBuf::new();
    let mut current_file_position: u64 = 0;
    let mut buffer = Vec::new();

    loop {
        tokio::select! {
            // Handle file events
            Some(event) = rx.next() => {
                let path = &event.paths[0];
                match event.kind {
                    notify::event::EventKind::Create(_) => {
                        let path = event.paths[0].clone();
                        if path.extension().map_or(false, |ext| ext == "dem") {
                            current_file_path = path.to_path_buf();
                            current_file_position = 0;
                        }
                    }
                    notify::event::EventKind::Modify(_) => {
                        if path == &current_file_path  {
                            let mut file = File::open(&current_file_path).await?;
                            buffer.clear();

                            file.seek(SeekFrom::Start(current_file_position)).await?;
                            let read_bytes = file.read_to_end(&mut buffer).await?;
                            current_file_position += read_bytes as u64;

                            if !buffer.is_empty() {
                                process_file_data(&buffer).await;
                            }
                        }
                    }
                    _ => {
                        tracing::error!("Unhandled event kind: {:?}", event.kind);
                    }
                }
            },
            // Handle metadata tick
            _ = metadata_tick.tick() => {
                // If there is a current file being watched, check its metadata
                if !current_file_path.as_os_str().is_empty()  {
                    let current_metadata = metadata(&current_file_path).await?;

                    // If the current file size is greater than the last known position, it's been updated
                    if current_metadata.len() > current_file_position {
                        let mut file = File::open(&current_file_path).await?;
                        buffer.clear();

                        file.seek(SeekFrom::Start(current_file_position)).await?;
                        let read_bytes = file.read_to_end(&mut buffer).await?;
                        current_file_position += read_bytes as u64;
                        if !buffer.is_empty() {
                            process_file_data(&buffer).await;
                        }
                    }
                }
            },
        }
    }
}

async fn process_file_data(data: &[u8]) {
    // Placeholder function
    tracing::debug!("Received data of length {}", data.len());
}
