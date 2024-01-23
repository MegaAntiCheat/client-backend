use std::{path::PathBuf, sync::Arc, time::Duration};

use event_loop::{Is, MessageSource, StateUpdater};
use tokio::sync::mpsc::{
    error::TryRecvError, unbounded_channel, Receiver, UnboundedReceiver, UnboundedSender,
};

use crate::io::filewatcher::{FileWatcher, FileWatcherCommand};

use self::console::RawConsoleOutput;

pub mod command_manager;
pub mod console;
pub mod handlers;
pub mod messages;
pub mod new_players;
pub mod steam_api;

pub struct Refresh;
impl<S> StateUpdater<S> for Refresh {}

pub async fn refresh_timer(interval: Duration) -> Receiver<Refresh> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);

    let mut interval = tokio::time::interval(interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    tokio::task::spawn(async move {
        loop {
            interval.tick().await;
            if let Ok(_) = tx.send(Refresh).await {
                continue;
            }

            tracing::error!("Couldn't send refresh message. Exiting refresh loop.");
        }
    });

    rx
}

pub struct ConsoleLog {
    recv: UnboundedReceiver<Arc<str>>,
    _send: UnboundedSender<FileWatcherCommand>,

    logged_error: bool,
}
impl<M: Is<RawConsoleOutput>> MessageSource<M> for ConsoleLog {
    fn next_message(&mut self) -> Option<M> {
        match self.recv.try_recv() {
            Ok(msg) => Some(RawConsoleOutput(msg).into()),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                if !self.logged_error {
                    tracing::error!("No more console messages coming.");
                    self.logged_error = true;
                }
                None
            }
        }
    }
}
impl ConsoleLog {
    pub async fn new(log_file_path: PathBuf) -> ConsoleLog {
        let (console_log_tx, console_log_rx) = unbounded_channel();
        let (console_rx, mut log_watcher) = FileWatcher::new(log_file_path, console_log_rx);
        tokio::task::spawn(async move {
            log_watcher.file_watch_loop().await;
        });

        ConsoleLog {
            recv: console_rx,
            _send: console_log_tx,
            logged_error: false,
        }
    }
}
