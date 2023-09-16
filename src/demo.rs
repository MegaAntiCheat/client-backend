use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::path::Path;
use std::path::PathBuf;
use std::thread;
use tokio::fs::metadata;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::sync::mpsc;
use tokio::time::interval;
use tokio::time::Duration;
use serde::{Serialize, Deserialize};

#[derive(Serialize, Deserialize, Debug)]
pub struct BigBrotherPacket {
    players: Vec<String>, // List of players on the server, array of SteamIDs
    ip: String, // IP address of the server
    sequence: i32, // A sequence number that starts at 0 and increments for each successive fragment
    demo: Option<Vec<u8>>, // Up-to-date copy of demo file. None for packet 0 as the demo wouldn't be flushed to disk yet.
}


pub async fn demo_loop(demo_path: PathBuf) -> anyhow::Result<()> {
    let (sync_tx, sync_rx) = std::sync::mpsc::channel::<Event>();
    let (tx, mut rx) = mpsc::channel(32);
    let config = Config::default().with_poll_interval(Duration::from_secs(2));

    let mut watcher: RecommendedWatcher = Watcher::new(
        Box::new(move |res: Result<Event, notify::Error>| {
            let _ = sync_tx.send(res.unwrap());
        }),
        config,
    )?;

    watcher.watch(demo_path.as_path(), RecursiveMode::NonRecursive)?;

    thread::spawn(move || loop {
        if let Ok(event) = sync_rx.recv() {
            let _ = tx.blocking_send(event); // This blocks in a separate thread so idgaf anymore
        }
    });

    let mut current_file_path = String::new();
    let mut current_file_position: u64 = 0;

    // Create a tick interval to periodically check metadata
    let mut metadata_tick = interval(Duration::from_secs(5));

    tracing::info!("Demo loop started");
    loop {
        tokio::select! {
            // Handle file events
            Some(event) = rx.recv() => {
                match event.kind {
                    notify::event::EventKind::Create(_) => {
                        let path = event.paths[0].clone();
                        if path.extension().map_or(false, |ext| ext == "dem") {
                            current_file_path = path.to_string_lossy().to_string();
                            current_file_position = 0;
                            tracing::info!("New .dem file created: {}", &current_file_path);
                        } else {
                            tracing::info!("Ignored file with non-.dem extension: {}", path.to_string_lossy());
                        }
                    }
                    notify::event::EventKind::Modify(_) => {
                        let path = event.paths[0].clone();
                        if path.to_string_lossy() == current_file_path {
                            let mut file = File::open(&current_file_path).await?;
                            let mut buffer = Vec::new();

                            file.seek(SeekFrom::Start(current_file_position)).await?;
                            let read_bytes = file.read_to_end(&mut buffer).await?;
                            current_file_position += read_bytes as u64;

                            if buffer.len() != 0{
                                process_file_data(buffer).await;
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
                if !current_file_path.is_empty() {
                    let current_metadata = metadata(&current_file_path).await?;
                    let current_size = current_metadata.len();

                    // If the current file size is greater than the last known position, it's been updated
                    if current_size > current_file_position {
                        let mut file = File::open(Path::new(&current_file_path)).await?;
                        let mut buffer = Vec::new();
                        file.seek(SeekFrom::Start(current_file_position)).await?;
                        let read_bytes = file.read_to_end(&mut buffer).await?;
                        current_file_position += read_bytes as u64;
                        if buffer.len() != 0{
                            process_file_data(buffer).await;
                        }
                    }
                }
            },
        }
    }
}

async fn process_file_data(data: Vec<u8>) {
    // Placeholder function
    tracing::info!("Received data of length {}", data.len());
}
