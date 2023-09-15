use notify::{Config, Event, RecommendedWatcher, RecursiveMode, Watcher};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use tokio::sync::mpsc::channel;
use tokio::time::sleep;

pub struct DemoManager {
    newest_file: Option<PathBuf>,
    dir_path: PathBuf,
    last_checked_time: SystemTime,
    known_files: HashSet<PathBuf>,
}

impl DemoManager {
    pub fn new(demo_path: PathBuf) -> Self {
        Self {
            newest_file: None,
            dir_path: demo_path,
            last_checked_time: SystemTime::UNIX_EPOCH,
            known_files: HashSet::new(),
        }
    }
}

fn async_watcher() -> notify::Result<(
    RecommendedWatcher,
    tokio::sync::mpsc::Receiver<notify::Result<Event>>,
)> {
    let (tx, rx) = channel(1);

    let watcher = RecommendedWatcher::new(
        move |res| {
            let tx = tx.clone();
            tokio::spawn(async move {
                tx.send(res).await.unwrap();
            });
        },
        Config::default(),
    )?;

    Ok((watcher, rx))
}

pub async fn demo_loop(demo_path: PathBuf) -> anyhow::Result<()> {
    let mut demo_manager = DemoManager::new(demo_path.clone());

    let (mut watcher, mut rx) = async_watcher()?;

    // Watch the directory for new files
    watcher
        .watch(&demo_manager.dir_path, RecursiveMode::NonRecursive)
        .map_err(|e| {
            tracing::error!("Failed to watch directory: {:?}", e);
            e
        })?;

    let mut current_watched_file = None;
    let mut last_modified_time = SystemTime::UNIX_EPOCH;
    let mut last_file_size: u64 = 0;

    loop {
        tokio::select! {
            _ = sleep(Duration::from_secs(3)) => {
                // Fallback logic in case notify fails
                tracing::debug!("Notify failed, checking for new demos");
            },
            event_option = rx.recv() => {
                match event_option {
                    Some(Ok(event)) => {
                        for path in event.paths {
                            // If its the currently watched file that changed
                            if Some(&path) == current_watched_file.as_ref() {
                                let metadata = match tokio::fs::metadata(&path).await {
                                    Ok(md) => md,
                                    Err(e) => {
                                        tracing::error!("Failed to get metadata: {:?}", e);
                                        continue;
                                    }
                                };
                                let current_modified_time = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
                                let current_file_size = metadata.len();

                                // Calculate time since last update and size change
                                let elapsed_time = current_modified_time
                                    .duration_since(last_modified_time)
                                    .unwrap_or_else(|_| Duration::from_secs(0));
                                let size_difference = current_file_size as i64 - last_file_size as i64;

                                let change = match size_difference {
                                    x if x > 0 => format!("increased by {} bytes", x),
                                    x if x < 0 => format!("decreased by {} bytes", x.abs()),
                                    _ => "remained the same".to_string(),
                                };

                                tracing::info!(
                                    "File has been updated. Time since last update: {:.2} seconds. File size {}.",
                                    elapsed_time.as_secs_f64(),
                                    change
                                );

                                last_modified_time = current_modified_time;
                                last_file_size = current_file_size;

                                continue;
                            }

                            // If it's a new file
                            if path.is_file() && path.extension() == Some("dem".as_ref()) {
                                let metadata = tokio::fs::metadata(&path).await.unwrap();
                                let modified_time = metadata.modified().unwrap();

                                if modified_time > demo_manager.last_checked_time {
                                    demo_manager.known_files.insert(path.clone());

                                    if demo_manager.newest_file.as_ref().map_or(true, |file| &path > file) {
                                        demo_manager.newest_file = Some(path.clone());
                                        current_watched_file = Some(path.clone());
                                        watcher.watch(&path, RecursiveMode::NonRecursive).unwrap();
                                    }
                                }
                            }
                        }
                    },
                    Some(Err(e)) => {
                        tracing::error!("Watch error: {:?}", e);
                    },
                    None => {
                        tracing::error!("Watch channel stopped");
                    }
                }
            }
        }
    }
}
