use std::fs::File;
use std::io;
use std::io::prelude::*;
use std::io::BufReader;
use std::io::SeekFrom;
use std::path::Path;
use std::path::PathBuf;
use std::time::Instant;

pub struct LogWatcher {
    filepath: Box<Path>,
    pos: u64,
    reader: BufReader<File>,
    last_activity: Instant,
}

impl LogWatcher {
    // Try to open this TF2 directory
    pub fn use_directory(mut dir: PathBuf) -> Result<LogWatcher, io::Error> {
        dir.push("tf/console.log");
        LogWatcher::register(dir)
    }

    /// Internally called by [use_directory]
    pub fn register(filepath: PathBuf) -> Result<LogWatcher, io::Error> {
        let f = match File::open(&filepath) {
            Ok(x) => {
                if let Ok(path) = filepath.clone().into_os_string().into_string() {
                    log::error!("Successfully opened log file: {}", path);
                } else {
                    log::error!("Successfully opened log file");
                }
                x
            }
            Err(err) => {
                if let Ok(path) = filepath.into_os_string().into_string() {
                    log::error!("Failed to open log file {}: {}", path, err);
                } else {
                    log::error!("Failed to open log file: {}", err);
                }
                return Err(err);
            }
        };

        let metadata = match f.metadata() {
            Ok(x) => x,
            Err(err) => {
                log::error!("Failed to get file metadata: {}", err);
                return Err(err);
            }
        };

        let mut reader = BufReader::new(f);
        let pos = metadata.len();
        if let Err(e) = reader.seek(SeekFrom::Start(pos)) {
            log::error!("Failed to seek in file: {}", e);
        }
        Ok(LogWatcher {
            filepath: filepath.into_boxed_path(),
            pos,
            reader,
            last_activity: Instant::now(),
        })
    }

    pub fn next_line(&mut self) -> Option<String> {
        let mut line = String::new();
        let resp = self.reader.read_line(&mut line);

        match resp {
            Ok(len) => {
                // Get next line
                if len > 0 {
                    self.pos += len as u64;
                    self.reader.seek(SeekFrom::Start(self.pos)).unwrap();
                    self.last_activity = Instant::now();
                    return Some(line.replace('\n', ""));
                }

                // Check if file has been shortened
                if self.reader.get_ref().metadata().unwrap().len() < self.pos {
                    log::warn!("Console.log file was reset");
                    self.pos = self.reader.get_ref().metadata().unwrap().len();
                    self.last_activity = Instant::now();
                }

                // Reopen the log file if nothing has happened for long enough in case the file has been replaced.
                let time = Instant::now().duration_since(self.last_activity);
                if time.as_secs() > 10 {
                    log::debug!("No activity in 10 seconds, reopening.");
                    let f = match File::open(&self.filepath) {
                        Ok(x) => x,
                        Err(_) => return None,
                    };

                    let metadata = match f.metadata() {
                        Ok(x) => x,
                        Err(_) => return None,
                    };

                    let mut reader = BufReader::new(f);
                    let pos = metadata.len();
                    reader.seek(SeekFrom::Start(pos)).unwrap();

                    self.pos = pos;
                    self.reader = reader;
                    self.last_activity = Instant::now();
                    return None;
                }

                self.reader.seek(SeekFrom::Start(self.pos)).unwrap();
                return None;
            }
            Err(err) => {
                log::error!("Logwatcher error: {}", err);
            }
        }

        None
    }
}
