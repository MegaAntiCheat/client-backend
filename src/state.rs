use anyhow::Result;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};
use tokio::sync::mpsc::UnboundedSender;

use crate::{io::Commands, player_records::PlayerRecords, server::Server, settings::Settings};

// Struct for shared state and lock helpers
pub struct Shared<S>(Arc<RwLock<S>>);

impl<S> Clone for Shared<S> {
    fn clone(&self) -> Self {
        Shared(self.0.clone())
    }
}

impl<S> Shared<S> {
    pub fn new(state: S) -> Shared<S> {
        Shared(Arc::new(RwLock::new(state)))
    }
    pub fn read(&self) -> RwLockReadGuard<S> {
        self.0.read().expect("State poisoned")
    }
    pub fn write(&self) -> RwLockWriteGuard<S> {
        self.0.write().expect("State poisoned")
    }
}

// State

pub struct State {
    pub log_file_state: Result<()>,
    pub rcon_state: Result<()>,
    pub command_issuer: UnboundedSender<Commands>,
    pub server: Server,
    pub settings: Settings,
}

impl State {
    pub fn new(
        settings: Settings,
        playerlist: PlayerRecords,
        command_issuer: UnboundedSender<Commands>,
    ) -> State {
        State {
            log_file_state: Ok(()),
            rcon_state: Ok(()),
            command_issuer,
            server: Server::new(playerlist),
            settings,
        }
    }
}
