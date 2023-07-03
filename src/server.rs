use std::{cell::OnceCell, sync::RwLock};

use crate::io::IOManager;

/// Global singleton of the server, so it can be accessed from the web API callbacks.
pub static SERVER: RwLock<Server> = RwLock::new(Server::new());

#[derive(Debug)]
pub struct Server {}

impl Server {
    pub const fn new() -> Server {
        Server {}
    }
}
