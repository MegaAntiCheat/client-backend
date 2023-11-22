use std::{path::PathBuf, time::Duration};

use anyhow::{anyhow, Context, Result};
use clap_lex::SeekFrom;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt},
    sync::mpsc::{error::TryRecvError, unbounded_channel, UnboundedReceiver, UnboundedSender},
};

/// Asynchronously watch a file and read any updates made to it
pub struct FileWatcher {
    sender: UnboundedSender<PathBuf>,
    receiver: UnboundedReceiver<String>,
}

impl FileWatcher {
    /// Start watching a file for updates in a dedicated [tokio::task]
    pub async fn new(path: PathBuf) -> FileWatcher {
        let (req_tx, resp_rx) = FileWatcherInner::new(path).await;

        FileWatcher {
            sender: req_tx,
            receiver: resp_rx,
        }
    }

    /// Change the path of the file to watch
    pub fn set_watched_file(&self, path: PathBuf) {
        self.sender.send(path).expect("File watcher loop ded");
    }

    /// Attempts to receive the next line. Returns [None] if there is none ready
    pub fn try_next_line(&mut self) -> Option<String> {
        match self.receiver.try_recv() {
            Ok(line) => Some(line),
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => panic!("File watcher loop ded"),
        }
    }

    /// Receives the next line. Since this just reading from a [UnboundedReceiver]
    /// it is cancellation safe.
    pub async fn next_line(&mut self) -> String {
        self.receiver.recv().await.expect("File watcher loop ded")
    }
}

struct OpenFile {
    /// Size of the file (in bytes) when it was last read
    pub last_size: u64,
    /// The file being watched
    pub file: File,
}

struct FileWatcherInner {
    /// Used to reopen the file for the next bulk read
    file_path: PathBuf,
    /// The file currently being watched
    open_file: Option<OpenFile>,

    request_recv: UnboundedReceiver<PathBuf>,
    response_send: UnboundedSender<String>,
}

impl FileWatcherInner {
    async fn new(path: PathBuf) -> (UnboundedSender<PathBuf>, UnboundedReceiver<String>) {
        let (req_tx, req_rx) = unbounded_channel();
        let (resp_tx, resp_rx) = unbounded_channel();

        let mut file_watcher = FileWatcherInner {
            file_path: path,
            open_file: None,

            request_recv: req_rx,
            response_send: resp_tx,
        };

        tokio::task::spawn(async move {
            file_watcher.file_watch_loop().await;
        });

        (req_tx, resp_rx)
    }

    async fn file_watch_loop(&mut self) {
        if let Err(e) = self.first_file_open().await {
            tracing::error!("Failed to open file {:?}: {:?}", &self.file_path, e);
            self.open_file = None;
        }

        loop {
            if let Ok(new_path) = self.request_recv.try_recv() {
                self.file_path = new_path;
                if let Err(e) = self.reopen_file().await {
                    tracing::error!("Failed to open new file {:?}: {:?}", self.file_path, e);
                }
            }

            match self.open_file {
                Some(_) => {
                    self.read_new_file_lines().await.ok();
                }
                None => {
                    self.reopen_file().await.ok();
                }
            }

            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }

    async fn first_file_open(&mut self) -> Result<()> {
        let open_file = self.reopen_file().await?;
        let meta = open_file.file.metadata().await?;
        open_file.file.seek(SeekFrom::Start(meta.len())).await?;
        open_file.last_size = meta.len();

        Ok(())
    }

    async fn reopen_file(&mut self) -> tokio::io::Result<&mut OpenFile> {
        let file = OpenOptions::new()
            .read(true)
            .write(false)
            .open(&self.file_path)
            .await?;

        self.open_file = Some(OpenFile { last_size: 0, file });

        Ok(self.open_file.as_mut().unwrap())
    }

    /// Attempts to read the new contents of the observed file and updates the internal state
    /// with any new lines that have been appended since last call.
    async fn read_new_file_lines(&mut self) -> Result<()> {
        if self.open_file.is_none() {
            return Err(anyhow!(
                "read_new_file_lines wasn't meant to be called when self.file is None"
            ));
        }
        let mut file = self.open_file.as_mut().unwrap();

        let meta =
            std::fs::metadata(&self.file_path).context("Failed to fetch metadata for log file.")?;

        // No new data
        if meta.len() == file.last_size || meta.len() == 0 {
            return Ok(());
        }

        // Reset if file has been remade (i.e. is shorter) and update state
        if meta.len() < file.last_size {
            tracing::warn!("File has shortened, the file may have been replaced. Reopening.");
            file = self
                .reopen_file()
                .await
                .context("Failed to reopen file after it was shortened.")?;
        }

        // Get new file contents
        let mut buff: Vec<u8> = Vec::new();
        let read_size = file
            .file
            .read_to_end(&mut buff)
            .await
            .context("Failed to read file.")?;

        file.last_size += read_size as u64;

        // If we expected there to be new data but we didn't read anything, reopen the file and try again.
        if read_size == 0 {
            tracing::warn!("Expected to read bytes but didn't get any, the file may have been replaced. Reopening.");
            file = self
                .reopen_file()
                .await
                .context("Failed to reopen file after not receiving any data.")?;
            buff.clear();
            file.last_size = file
                .file
                .read_to_end(&mut buff)
                .await
                .context("Failed to read file.")? as u64;
        }

        // Send newly read lines over channel
        let data_str = String::from_utf8_lossy(&buff);
        data_str
            .lines()
            .filter(|x| !x.trim().is_empty())
            .for_each(|l| {
                self.response_send
                    .send(l.to_string())
                    .expect("Main loop ded?");
            });

        Ok(())
    }
}
