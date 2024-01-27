use std::{path::PathBuf, sync::Arc, time::Duration};

use event_loop::{Is, MessageSource, StateUpdater};
use tokio::sync::mpsc::{
    error::TryRecvError, unbounded_channel, Receiver, UnboundedReceiver, UnboundedSender,
};

use crate::console::RawConsoleOutput;
use crate::io::filewatcher::{FileWatcher, FileWatcherCommand};
use crate::state::MACState;

#[derive(Debug, Clone, Copy)]
pub struct Refresh;
impl StateUpdater<MACState> for Refresh {
    fn update_state(self, state: &mut MACState) {
        state.players.refresh();
    }
}

pub async fn emit_on_timer<M: 'static + Send>(
    interval: Duration,
    emit: fn() -> M,
) -> Box<Receiver<M>> {
    let (tx, rx) = tokio::sync::mpsc::channel(1);

    let mut interval = tokio::time::interval(interval);
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

    tokio::task::spawn(async move {
        loop {
            interval.tick().await;
            if let Ok(_) = tx.send(emit()).await {
                continue;
            }

            tracing::error!("Couldn't send refresh message. Exiting refresh loop.");
        }
    });

    Box::new(rx)
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
