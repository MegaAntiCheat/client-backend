use notify::{Event, RecursiveMode, Watcher, RecommendedWatcher};
use std::path::PathBuf;
use std::thread;
use tokio::fs::File;
use tokio::io::{AsyncReadExt, AsyncSeekExt, SeekFrom};
use tokio::sync::mpsc;

pub async fn demo_loop(demo_path: PathBuf) -> anyhow::Result<()> {
    let (sync_tx, sync_rx) = std::sync::mpsc::channel::<Event>();
    
    let (tx, mut rx) = mpsc::channel(32);

    let mut watcher: RecommendedWatcher  = notify::recommended_watcher(
        Box::new(move |res: Result<Event, notify::Error>| {
            let _ = sync_tx.send(res.unwrap());
        })
    )?;

    watcher.watch(demo_path.as_path(), RecursiveMode::NonRecursive)?;

    thread::spawn(move || loop {
        if let Ok(event) = sync_rx.recv() {
            let _ = tx.blocking_send(event); // This blocks in a separate thread so idgaf anymore
        }
    });

    let mut current_file_path = String::new();
    let mut current_file_position: u64 = 0;

    while let Some(event) = rx.recv().await {
        match event.kind {
            notify::event::EventKind::Create(notify::event::CreateKind::File) => {
                let path = event.paths[0].clone();
                current_file_path = path.to_string_lossy().to_string();
                current_file_position = 0;
                tracing::info!("New .dem file created: {}", &current_file_path);
            }
            notify::event::EventKind::Modify(notify::event::ModifyKind::Data(_)) => {
                let path = event.paths[0].clone();
                if path.to_string_lossy() == current_file_path {
                    let mut file = File::open(&current_file_path).await?;
                    let mut buffer = Vec::new();

                    file.seek(SeekFrom::Start(current_file_position)).await?;
                    let read_bytes = file.read_to_end(&mut buffer).await?;
                    current_file_position += read_bytes as u64;

                    process_file_data(buffer).await;
                    tracing::info!("File {} updated", &current_file_path);
                }
            }
            _ => {}
        }
    }
    Ok(())
}

async fn process_file_data(data: Vec<u8>) {
    // Placeholder function
    tracing::info!("Received data of length {}", data.len());
}
