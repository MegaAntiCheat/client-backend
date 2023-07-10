use std::{
    ops::{Deref, DerefMut},
    sync::{RwLock, RwLockReadGuard, RwLockWriteGuard},
};

use crate::{server::Server, settings::Settings};

// State singleton and lock helpers

static STATE: RwLock<Option<State>> = RwLock::new(None);

pub struct StateReadLock<'a> {
    lock: RwLockReadGuard<'a, Option<State>>,
}

impl Deref for StateReadLock<'_> {
    type Target = State;

    fn deref(&self) -> &Self::Target {
        self.lock.as_ref().unwrap()
    }
}

pub struct StateWriteLock<'a> {
    lock: RwLockWriteGuard<'a, Option<State>>,
}

impl Deref for StateWriteLock<'_> {
    type Target = State;

    fn deref(&self) -> &Self::Target {
        self.lock.as_ref().unwrap()
    }
}
impl DerefMut for StateWriteLock<'_> {
    fn deref_mut(&mut self) -> &mut State {
        self.lock.as_mut().unwrap()
    }
}

// State

#[derive(Debug)]
pub struct State {
    pub log_file_state: std::io::Result<()>,
    pub rcon_state: Result<(), rcon::Error>,
    pub server: Server,
    pub settings: Settings,
}

impl State {
    pub fn initialize_state(state: State) {
        *STATE.write().unwrap() = Some(state);
    }

    pub fn read_state() -> StateReadLock<'static> {
        StateReadLock {
            lock: STATE.read().unwrap(),
        }
    }

    pub fn write_state() -> StateWriteLock<'static> {
        StateWriteLock {
            lock: STATE.write().unwrap(),
        }
    }

    pub fn new(settings: Settings) -> State {
        State {
            log_file_state: Ok(()),
            rcon_state: Ok(()),
            server: Server::new(),
            settings,
        }
    }
}
