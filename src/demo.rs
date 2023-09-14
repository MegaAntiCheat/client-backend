use std::collections::HashSet;
use std::fs::{self, DirEntry};
use std::io;
use std::path::{Path, PathBuf};

use std::time::{Duration, SystemTime};
use tokio::fs::metadata;
use tokio::time::sleep;

pub struct DemoManager {
    newest_file: Option<PathBuf>,
    dir_path: Option<PathBuf>,
    last_checked_time: SystemTime,
    known_files: HashSet<PathBuf>,
}

impl DemoManager {
    pub fn new(demo_path: Option<PathBuf>) -> Self {
        Self {
            newest_file: None,
            dir_path: demo_path,
            last_checked_time: SystemTime::UNIX_EPOCH,
            known_files: HashSet::new(),
        }
    }

    pub async fn find_newest_dem_file(&mut self) -> io::Result<Option<PathBuf>> {
        let mut newest_file_time = self
            .newest_file
            .as_ref()
            .and_then(|path| fs::metadata(path).ok()?.modified().ok());

        let mut newest_file_path = None;

        for entry in fs::read_dir(self.dir_path)? {
            let path = entry?.path();

            if !self.known_files.contains(&path)
                && path.is_file()
                && path.extension() == Some("dem".as_ref())
            {
                let metadata = fs::metadata(&path)?;
                let modified_time = metadata.modified()?;

                if modified_time > self.last_checked_time {
                    self.known_files.insert(path.clone());

                    if newest_file_time.map_or(true, |time| modified_time > time) {
                        newest_file_time = Some(modified_time);
                        newest_file_path = Some(path);
                    }
                }
            }
        }

        self.last_checked_time = SystemTime::now();
        if let Some(new_path) = newest_file_path {
            self.newest_file = Some(new_path);
        }

        Ok(self.newest_file.clone())
    }
}

async fn monitor_file(file_path: PathBuf) {
    // Check if the file exists
    if !file_path.exists() {
        tracing::debug!("File {:?} does not exist!", file_path);
        return;
    }

    // Initialize last modified time and file size
    let mut last_modified_time = match get_last_modified_time(&file_path).await {
        Ok(time) => time,
        Err(e) => {
            tracing::debug!("Failed to get last modified time: {}", e);
            return;
        }
    };

    let mut last_file_size = match get_file_size(&file_path).await {
        Ok(size) => size,
        Err(e) => {
            tracing::debug!("Failed to get file size: {}", e);
            return;
        }
    };

    loop {
        // Get current modified time and file size
        let current_modified_time = match get_last_modified_time(&file_path).await {
            Ok(time) => time,
            Err(e) => {
                tracing::debug!("Failed to get last modified time: {}", e);
                continue;
            }
        };

        let current_file_size = match get_file_size(&file_path).await {
            Ok(size) => size,
            Err(e) => {
                tracing::debug!("Failed to get file size: {}", e);
                continue;
            }
        };

        // Check for updates
        if current_modified_time != last_modified_time {
            let elapsed_time = current_modified_time
                .duration_since(last_modified_time)
                .unwrap();
            let size_difference = current_file_size as i64 - last_file_size as i64;

            let change = match size_difference {
                x if x > 0 => format!("increased by {} bytes", x),
                x if x < 0 => format!("decreased by {} bytes", x.abs()),
                _ => "remained the same".to_string(),
            };

            tracing::debug!(
                "File has been updated. Time since last update: {:.2} seconds. File size {}.",
                elapsed_time.as_secs_f64(),
                change
            );

            last_modified_time = current_modified_time;
            last_file_size = current_file_size;
        }

        sleep(Duration::from_secs(1)).await;
    }
}

async fn get_last_modified_time(file_path: &PathBuf) -> std::io::Result<SystemTime> {
    metadata(file_path).await?.modified()
}

async fn get_file_size(file_path: &PathBuf) -> std::io::Result<u64> {
    Ok(metadata(file_path).await?.len())
}
