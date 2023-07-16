#![allow(non_upper_case_globals)]
#![allow(unused_variables)]

use std::{
    collections::VecDeque,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    time::SystemTime,
};

use anyhow::{Context, Result};

pub struct FileWatcher {
    /// Used to reopen the file for the next bulk read
    file_path: PathBuf,
    /// Cursor position after the last read
    cpos: usize,
    /// Data from last file read, split on 0xA <u8> bytes
    lines_buf: VecDeque<String>,
    /// system time of the last time this file was read (tracked by `file modified` timestamp,
    /// will be time of UNIX EPOCH if not implemented by the host OS. Would this ever happen?)
    last_read: Option<SystemTime>,
}

impl FileWatcher {
    pub fn use_directory(mut dir: PathBuf) -> Result<FileWatcher> {
        dir.push("tf/console.log");
        FileWatcher::new(dir)
    }

    pub fn new(path: PathBuf) -> Result<FileWatcher> {
        let file = FileWatcher::open_file(&path)?;
        let meta = file
            .metadata()
            .context("File didn't have metadata associated")?;
        let pos = meta.len();
        let last = meta.modified().ok();
        Ok(FileWatcher {
            file_path: path,
            cpos: pos as usize,
            lines_buf: VecDeque::new(),
            last_read: last,
        })
    }

    fn open_file<P: AsRef<Path>>(filepath: P) -> Result<File> {
        File::open(filepath).context("Failed to open log file")
    }

    /// Attempts to read the new contents of the observed file and updates the internal state
    /// with any new lines that have been appended since last call.
    fn read_new_file_lines(&mut self) -> Result<()> {
        // Get file and metadata
        let mut file =
            FileWatcher::open_file(&self.file_path).context("Failed to reopen log file")?;
        let meta = file
            .metadata()
            .context("File didn't have associated metadata")?;

        // Check it's been modified
        if let Some(last_read) = self.last_read {
            if meta.modified().context("Last read time has diappeared")? <= last_read {
                // Nothing to update
                return Ok(());
            }
        } else {
            log::error!("No stored last mod time.");
        }

        // Get new file contents
        let mut buff: Vec<u8> = Vec::new();
        let _ = file.read_to_end(&mut buff);
        let mut start_idx = self.cpos;

        // Reset if file has been remade (i.e. is shorter) and update state
        if buff.len() < self.cpos {
            start_idx = 0;
        }

        self.cpos = buff.len() - start_idx;
        self.last_read = meta.modified().ok();

        // Get strings
        let data_str = String::from_utf8_lossy(&buff[start_idx..]);
        self.lines_buf.extend(
            data_str
                .lines()
                .filter(|x| !x.trim().is_empty())
                .map(str::to_string),
        );

        Ok(())
    }

    pub fn get_line(&mut self) -> Option<String> {
        if self.lines_buf.is_empty() {
            if let Err(e) = self.read_new_file_lines() {
                log::error!("Failed to read log file: {}", e);
            }
        }

        self.lines_buf.pop_front()
    }
}
