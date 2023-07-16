#![allow(non_upper_case_globals)]
#![allow(unused_variables)]

use clap_lex::OsStrExt;
use std::{
    collections::VecDeque,
    ffi::OsString,
    fs::File,
    io::{self, Cursor, Read},
    path::{Path, PathBuf},
    time::SystemTime,
};
#[cfg(target_os = "linux")]
mod utf16_support {
    use std::{
        ffi::OsString,
        os::unix::ffi::OsStringExt,
    };
    pub fn from_bytes(buf: &[u8]) -> OsString {
        OsString::from_vec(&buf.to_vec())
    }
}

#[cfg(target_os = "windows")]
mod utf16_support {
    use std::{
        ffi::{OsStr, OsString},
        os::windows::{ffi::OsStringExt, prelude::OsStrExt},
    };

    /// Rust jank to handle windows Unicode https://gist.github.com/sunnyone/e660fe7f73e2becd4b2c
    fn to_wide_chars(s: &str) -> Vec<u16> {
        OsStr::new(s)
            .encode_wide()
            .chain(Some(0).into_iter())
            .collect::<Vec<_>>()
    }

    /// Convert a buf of bytes to a Windows unicode-supporting OsString using unsafe string conversion
    pub fn from_bytes(buf: &[u8]) -> OsString {
        unsafe {
            OsStringExt::from_wide(&to_wide_chars(&String::from_utf8_unchecked(buf.to_vec())))
        }
        ///// This is the alternative (i think this works?)
        // unsafe {
        //     let conv = String::from_utf8_unchecked(buf.to_vec());
        //     if let Ok(res) = OsString::from_str(&conv) {
        //         res
        //     } else {
        //         OsString::new()
        //     }
        // }
    }
}

/// Used to shuttle data out of read_lines into get_line
struct ReadMD(Option<SystemTime>, Cursor<Vec<u8>>);

pub struct FileWatcher {
    /// Used to reopen the file for the next bulk read
    pub file_path: Box<Path>,
    /// Cursor position after the last read
    pub cpos: u64,
    /// Data from last file read, split on 0xA <u8> bytes
    pub lines_buf: VecDeque<OsString>,
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

            let data_str: OsString = utf16_support::from_bytes(&data);

            let mut lines: VecDeque<OsString> = VecDeque::new();
            let iter = data_str.split("\n");

            for os_str in iter {
                if os_str.to_string_lossy().is_empty() || os_str.to_string_lossy().trim().is_empty()
                {
                    continue;
                }
                lines.push_back(os_str.to_os_string());
            }

            self.lines_buf = lines;
        }
    }

    pub fn get_line(&mut self) -> Option<OsString> {
        if self.lines_buf.is_empty() {
            self.read_lines();
        }

        self.lines_buf.pop_front()
    }
}
