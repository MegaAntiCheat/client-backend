#![allow(non_upper_case_globals)]
#![allow(unused_variables)]

use std::{
    collections::VecDeque,
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom},
    path::PathBuf,
};

use anyhow::{Context, Result};

pub struct FileWatcher {
    /// Used to reopen the file for the next bulk read
    file_path: PathBuf,
    /// Data from last file read, split on 0xA <u8> bytes
    lines_buf: VecDeque<String>,
    /// Size of the file (in bytes) when it was last read
    last_size: usize,
    /// The file being watched
    file: File,
}

impl FileWatcher {
    pub fn new(path: PathBuf) -> Result<FileWatcher> {
        let mut file = OpenOptions::new()
            .read(true)
            .write(false)
            .open(&path)
            .context("Failed to open file to watch.")?;
        file.seek(SeekFrom::End(0))?;
        let meta = file
            .metadata()
            .context("File didn't have metadata associated")?;
        let last = meta.len();
        Ok(FileWatcher {
            file_path: path,
            lines_buf: VecDeque::new(),
            last_size: last as usize,
            file,
        })
    }

    /// Attempts to read the new contents of the observed file and updates the internal state
    /// with any new lines that have been appended since last call.
    fn read_new_file_lines(&mut self) -> Result<()> {
        let meta =
            std::fs::metadata(&self.file_path).context("Failed to fetch metadata for log file.")?;

        // No new data
        if meta.len() as usize == self.last_size || meta.len() == 0 {
            return Ok(());
        }

        let mut start_idx = self.last_size;

        // Reset if file has been remade (i.e. is shorter) and update state
        if (meta.len() as usize) < self.last_size {
            tracing::warn!("File has shortened, the file may have been replaced. Reopening."); 
            start_idx = 0;
            self.file = OpenOptions::new()
                .read(true)
                .write(false)
                .open(&self.file_path)
                .context("Failed to reopen file after it was shortened.")?;
        }

        self.last_size = meta.len() as usize;
        
        // Get new file contents
        let mut buff: Vec<u8> = Vec::new();
        let read_size = self.file.read_to_end(&mut buff).context("Failed to read file.")?;
        let expected_size = self.last_size - start_idx;

        // If the length of the data we received is less than we expected, reopen the file and try again
        if read_size < expected_size {
            tracing::warn!("Expected to read {expected_size} bytes but only read {read_size}, the file may have been replaced. Reopening.");
            self.file = OpenOptions::new()
                .read(true)
                .write(false)
                .open(&self.file_path)
                .context("Failed to reopen file after receiving less data than expected.")?;
            buff.clear();
            let _ = self.file.read_to_end(&mut buff).context("Failed to read file.")?;
        }

        let data_str = String::from_utf8_lossy(&buff);
        self.lines_buf.extend(
            data_str
                .lines()
                .filter(|x| !x.trim().is_empty())
                .map(str::to_string),
        );

        Ok(())
    }

    /// Return the next
    pub fn get_line(&mut self) -> Result<Option<String>> {
        if self.lines_buf.is_empty() {
            self.read_new_file_lines()
                .context("Failed to read new file lines.")?
        }

        Ok(self.lines_buf.pop_front())
    }
}
