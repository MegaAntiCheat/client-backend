use std::collections::HashSet;
use anyhow::Result;
use std::path::PathBuf;

use std::time::{Duration, SystemTime};
use tokio::fs::metadata;
use tokio::time::sleep;

use crate::state::State;


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

    pub async fn find_newest_dem_file(&mut self) -> Result<Option<PathBuf>> {
        let dir_path =  &self.dir_path;
        let mut dir = tokio::fs::read_dir(dir_path).await?;

        let newest_file_metadata = match &self.newest_file {
            Some(path) => metadata(path).await.ok(),
            None => None,
        };

        let mut newest_file_time = newest_file_metadata.and_then(|meta| meta.modified().ok());

        let mut newest_file_path = None;

        while let Some(entry) = dir.next_entry().await? {
            let path = entry.path();
            
            if !self.known_files.contains(&path)
                && path.is_file()
                && path.extension() == Some("dem".as_ref())
            {
                let metadata = tokio::fs::metadata(&path).await?;
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


pub async fn demo_loop(demo_path: PathBuf) {
    let mut demo_manager = DemoManager::new(demo_path);

    loop {
        match demo_manager.find_newest_dem_file().await {
            Ok(Some(path)) => {
                tracing::debug!("Found new demo file: {:?}", path);
                monitor_file(&mut demo_manager, path).await;
            }
            Ok(None) => {}
            Err(e) => {
                tracing::error!("Failed to find new demo file: {}", e);
            }
        }

        sleep(Duration::from_secs(5)).await;
    }
}

pub async fn monitor_file(demo_manager: &mut DemoManager, mut file_path: PathBuf) {
    // Check if the file exists
    if !file_path.exists() {
        tracing::debug!("File {:?} does not exist!", file_path);
        return;
    }

    // Initialize last modified time and file size
    let mut last_modified_time = match get_last_modified_time(&file_path).await {
        Ok(time) => time,
        Err(e) => {
            tracing::warn!("Failed to get last modified time: {}", e);
            return;
        }
    };

    let mut last_file_size = match get_file_size(&file_path).await {
        Ok(size) => size,
        Err(e) => {
            tracing::warn!("Failed to get file size: {}", e);
            return;
        }
    };


    loop {
        // Get current modified time and file size
        let current_modified_time = match get_last_modified_time(&file_path).await {
            Ok(time) => time,
            Err(e) => {
                tracing::warn!("Failed to get last modified time: {}", e);
                continue;
            }
        };

        let current_file_size = match get_file_size(&file_path).await {
            Ok(size) => size,
            Err(e) => {
                tracing::warn!("Failed to get file size: {}", e);
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

            tracing::info!(
                "File has been updated. Time since last update: {:.2} seconds. File size {}.",
                elapsed_time.as_secs_f64(),
                change
            );

            last_modified_time = current_modified_time;
            last_file_size = current_file_size;
        } else {
            // File hasn't changed

            if let Ok(Some(new_file)) = demo_manager.find_newest_dem_file().await {
                if new_file != file_path {
                    tracing::info!("Newer file found: {:?}. Switching to monitor this file.", new_file);
                    file_path = new_file;
                }
            }
            sleep(Duration::from_secs(3)).await;
        }

        sleep(Duration::from_secs(3)).await;
    }
}

async fn get_last_modified_time(file_path: &PathBuf) -> Result<SystemTime> {
    Ok(metadata(file_path).await?.modified()?)
}

async fn get_file_size(file_path: &PathBuf) -> Result<u64> {
    Ok(metadata(file_path).await?.len())
}
