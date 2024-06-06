use std::{path::PathBuf, time::Duration, fs::metadata};

use anyhow::{anyhow, Context, Result};
use clap_lex::SeekFrom;
use tokio::{
    fs::{File, OpenOptions},
    io::{AsyncReadExt, AsyncSeekExt},
    sync::mpsc::{unbounded_channel, UnboundedReceiver, UnboundedSender},
};

struct OpenFile {
    /// Size of the file (in bytes) when it was last read
    pub last_size: u64,
    /// The file being watched
    pub file: File,
}

pub struct FileWatcher {
    /// Used to reopen the file for the next bulk read
    file_path: PathBuf,
    /// The file currently being watched
    open_file: Option<OpenFile>,
    response_send: UnboundedSender<String>,
}

impl FileWatcher {
    #[must_use]
    pub fn new(path: PathBuf) -> (UnboundedReceiver<String>, Self) {
        let (resp_tx, resp_rx) = unbounded_channel();

        let file_watcher = Self {
            file_path: path,
            open_file: None,

            response_send: resp_tx,
        };

        (resp_rx, file_watcher)
    }

    /// Start the file watcher loop. This will block until the channel is
    /// closed, so usually it should be spawned in a separate `tokio::task`
    pub async fn file_watch_loop(&mut self) {
        if let Err(e) = self.first_file_open().await {
            tracing::error!("Failed to open file {:?}: {:?}", &self.file_path, e);
            self.open_file = None;
        }

        loop {
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

        Ok(self.open_file.as_mut().expect("Just check set it to some."))
    }

    /// Attempts to read the new contents of the observed file and updates the
    /// internal state with any new lines that have been appended since last
    /// call.
    /// 
    /// Assumptions: `self.open_file` is both Some and valid 
    async fn read_new_file_lines(&mut self) -> Result<()> {
        let Some(mut file) = self.open_file.as_mut() else {
            return Err(anyhow!("this function is uncallable when self.file is None!"));
        };

        let meta =
            metadata(&self.file_path).context("Failed to fetch metadata for log file.")?;

        // No new data
        if meta.len() == file.last_size || meta.len() == 0 {
            return Ok(());
        }

        // Reset if file has been remade (i.e. is shorter) and update state
        if meta.len() < file.last_size {
            // PR Lilith/sourcemod-enhancement: I don't think this should warn. It only has ever
            // appeared when people actually open TF2 and it causes more confusion. Made debug instead of warn.
            tracing::debug!("Detected new instance of file, reading from start.");
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

        // If we expected there to be new data but we didn't read anything, reopen the
        // file and try again.
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
                self.response_send.send(l.into()).expect("Main loop ded?");
            });

        Ok(())
    }
}
