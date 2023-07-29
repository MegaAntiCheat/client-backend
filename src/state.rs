use anyhow::Result;
use std::{
    ops::{Deref, DerefMut},
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{player_records::PlayerRecords, server::Server, settings::Settings};

// State singleton and lock helpers

static STATE: RwLock<Option<State>> = RwLock::new(None);

pub struct StateReadLock<'a> {
    lock: RwLockReadGuard<'a, Option<State>>,
}

impl Deref for StateReadLock<'_> {
    type Target = State;

    fn deref(&self) -> &Self::Target {
        self.lock.as_ref().expect("State lock")
    }
}

pub struct StateWriteLock<'a> {
    lock: RwLockWriteGuard<'a, Option<State>>,
}

impl Deref for StateWriteLock<'_> {
    type Target = State;

    fn deref(&self) -> &Self::Target {
        self.lock.as_ref().expect("State lock")
    }
}
impl DerefMut for StateWriteLock<'_> {
    fn deref_mut(&mut self) -> &mut State {
        self.lock.as_mut().expect("State lock")
    }
}

// State

pub struct State {
    pub log_file_state: Result<()>,
    pub rcon_state: Result<()>,
    pub server: Server,
    pub settings: Settings,
}

impl State {
    pub fn initialize_state(state: State) {
        *STATE.write().expect("State lock") = Some(state);
    }

    pub fn read_state() -> StateReadLock<'static> {
        StateReadLock {
            lock: STATE.read().expect("State lock"),
        }
    }

    pub fn write_state() -> StateWriteLock<'static> {
        StateWriteLock {
            lock: STATE.write().expect("State lock"),
        }
    }

    pub fn new(settings: Settings, playerlist: PlayerRecords) -> State {
        State {
            log_file_state: Ok(()),
            rcon_state: Ok(()),
            server: Server::new(playerlist),
            settings,
        }
    }
}
