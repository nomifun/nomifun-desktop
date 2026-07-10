use std::{
    collections::HashMap,
    sync::{Arc, RwLock},
};

use crate::{SessionId, supervisor::Session};

pub(crate) struct Registry {
    sessions: RwLock<HashMap<SessionId, Arc<Session>>>,
}

impl Registry {
    pub(crate) fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    pub(crate) fn insert(&self, session_id: SessionId, session: Arc<Session>) {
        self.sessions
            .write()
            .expect("execution registry lock is poisoned")
            .insert(session_id, session);
    }

    pub(crate) fn get(&self, session_id: &SessionId) -> Option<Arc<Session>> {
        self.sessions
            .read()
            .expect("execution registry lock is poisoned")
            .get(session_id)
            .cloned()
    }
}
