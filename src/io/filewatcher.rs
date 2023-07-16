#![allow(non_upper_case_globals)]
#![allow(unused_variables)]

use std::{
    collections::VecDeque,
    fs::File,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
    time::SystemTime,
};

/// Used to shuttle data out of read_lines into get_line
struct ReadMD(Option<SystemTime>, Cursor<Vec<u8>>);

pub struct FileWatcher {
    /// Used to reopen the file for the next bulk read
    pub file_path: Box<Path>,
    /// Cursor position after the last read
    pub cpos: u64,
    /// Data from last file read, split on 0xA <u8> bytes
    pub lines_buf: VecDeque<String>,
    /// system time of the last time this file was read (tracked by `file modified` timestamp,
    /// will be time of UNIX EPOCH if not implemented by the host OS. Would this ever happen?)
    pub last_read: Option<SystemTime>,
}

impl FileWatcher {
    pub fn use_directory(mut dir: PathBuf) -> Result<FileWatcher, io::Error> {
        dir.push("tf/console.log");
        FileWatcher::new(dir)
    }

    pub fn new(path: PathBuf) -> Result<FileWatcher, io::Error> {
        match FileWatcher::open_file(path.clone()) {
            Ok(file) => {
                let pos = file.metadata().unwrap().len();
                let last = file.metadata().unwrap().modified().ok();
                Ok(FileWatcher {
                    file_path: path.into_boxed_path(),
                    cpos: pos,
                    lines_buf: VecDeque::new(),
                    last_read: last,
                })
            }
            Err(err) => Err(err),
        }
    }

    fn open_file(filepath: PathBuf) -> Result<File, io::Error> {
        match File::open(&filepath) {
            Ok(x) => Ok(x),
            Err(err) => {
                if let Ok(path) = filepath.into_os_string().into_string() {
                    log::error!("Failed to open log file {}: {}", path, err);
                } else {
                    log::error!("Failed to open log file: {}", err);
                }
                Err(err)
            }
        }
    }

    fn seek_to_pos(&self) -> Option<ReadMD> {
        if let Ok(mut file) = FileWatcher::open_file(self.file_path.clone().into_path_buf()) {
            if let Ok(modified) = file.metadata().unwrap().modified() {
                if let Some(last_read) = self.last_read {
                    if modified <= last_read {
                        return None;
                    }
                } else {
                    log::error!("No stored last mod time.");
                }
            } else {
                log::error!("Unable to read modification time of watched file.");
            }
            let mut buff: Vec<u8> = Vec::new();
            let _ = file.read_to_end(&mut buff);

            let mut reader = Cursor::new(buff);
            reader.set_position(self.cpos);

            if let Ok(mod_time) = file.metadata().unwrap().modified() {
                Some(ReadMD(Some(mod_time), reader))
            } else {
                Some(ReadMD(None, reader)) // TODO: fix this grossness
            }
        } else {
            None
        }
    }

    fn read_lines(&mut self) {
        if let Some(mut read_md) = self.seek_to_pos() {
            self.last_read = read_md.0;

            let mut data: Vec<u8> = Vec::new();
            if let Ok(bytes_read) = read_md.1.read_to_end(&mut data) {
                self.cpos += bytes_read as u64;
            } else {
                log::error!("Could not read file data!");
                return;
            }

            let data_str = String::from_utf8_lossy(&data);
            let mut lines: VecDeque<String> = VecDeque::new();

            self.lines_buf.extend(
                data_str
                    .lines()
                    .filter(|x| !(x.trim().is_empty()))
                    .map(str::to_string),
            );
        }
    }

    pub fn get_line(&mut self) -> Option<String> {
        if self.lines_buf.is_empty() {
            self.read_lines();
        }
        // log::warn!("Got data: {:?}", self.lines_buf.front());
        self.lines_buf.pop_front()
    }
}
