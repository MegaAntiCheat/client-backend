use anyhow::anyhow;
use serde::Deserialize;
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};
use watchman_client::prelude::*;
use watchman_client::SubscriptionData;

query_result_type! {
    struct NameOnly {
        name: NameField,
    }
}

pub struct DemoManager {
    newest_file: Option<PathBuf>,
    last_checked_time: SystemTime,
    known_files: HashSet<PathBuf>,
}

impl DemoManager {
    pub fn new() -> Self {
        Self {
            newest_file: None,
            last_checked_time: SystemTime::UNIX_EPOCH,
            known_files: HashSet::new(),
        }
    }
}

pub async fn demo_loop(demo_path: PathBuf) -> anyhow::Result<()> {
    let mut demo_manager = DemoManager::new();

    let client = Connector::new().connect().await?;

    let subscribe_request = SubscribeRequest {
        since: None, // or Some(last_known_clock)
        relative_root: None,
        expression: Some(Expr::Suffix(vec![PathBuf::from("dem")])),
        fields: vec!["name"],
        empty_on_fresh_instance: false,
        case_sensitive: false,
        defer_vcs: false,
        defer: vec![],
        drop: vec![],
    };

    let resolved = client
        .resolve_root(CanonicalPath::canonicalize(&demo_path)?)
        .await?;

    let (mut demo_subscription, _) = client
        .subscribe::<NameOnly>(&resolved, subscribe_request)
        .await?;

    let mut current_watched_file = None;
    let mut last_modified_time = SystemTime::UNIX_EPOCH;
    let mut last_file_size: u64 = 0;

    loop {
        match demo_subscription.next().await {
            Ok(SubscriptionData::FilesChanged(query_result)) => {
                if let Some(files) = query_result.files {
                    for name_only_item in files.iter() {
                        let path_str = name_only_item.name.to_string_lossy().into_owned();
                        let path = PathBuf::from(path_str);

                        // If it's a new file
                        if !demo_manager.known_files.contains(&path) {
                            let metadata = tokio::fs::metadata(&path).await?;
                            let modified_time = metadata.modified().unwrap();

                            // Update last_checked_time and other metadata
                            demo_manager.last_checked_time = modified_time;
                            demo_manager.known_files.insert(path.clone());

                            if demo_manager
                                .newest_file
                                .as_ref()
                                .map_or(true, |file| &path > file)
                            {
                                demo_manager.newest_file = Some(path.clone());
                                current_watched_file = Some(path.clone());
                            }
                        }

                        // If its the currently watched file that changed
                        if Some(&path) == current_watched_file.as_ref() {
                            let metadata = match tokio::fs::metadata(&path).await {
                                Ok(md) => md,
                                Err(e) => {
                                    tracing::error!("Failed to get metadata: {:?}", e);
                                    continue;
                                }
                            };
                            let current_modified_time =
                                metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
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
                    }
                }
            }
            Ok(SubscriptionData::Canceled) => {
                tracing::error!("Subscription was canceled");
                break Err(anyhow!("Subscription was canceled"));
            }
            Ok(data) => {
                tracing::error!("Unexpected subscription data: {:?}", data);
            }
            Err(e) => {
                tracing::error!("Subscription error: {:?}", e);
            }
        }
    }
}
