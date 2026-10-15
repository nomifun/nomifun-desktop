//! Revocable ownership of real, not-yet-admitted native popup pages.
//! This reducer has no AppKit dependency so admission races can be tested
//! without starting a browser or replacing native close acknowledgements.
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ticket {
    pub id: u64,
    epoch: u64,
    pub document_generation: u64,
    pub navigation_sequence: u64,
}
struct Candidate<T> { ticket: Ticket, child: T }
pub(crate) struct Admission<T> {
    next: u64,
    epoch: u64,
    paused: bool,
    closed: bool,
    candidates: BTreeMap<u64, Candidate<T>>,
}
impl<T> Default for Admission<T> {
    fn default() -> Self {
        Self { next: 0, epoch: 0, paused: false, closed: false, candidates: BTreeMap::new() }
    }
}
impl<T> Admission<T> {
    pub fn reserve(&mut self, child: T, document_generation: u64, navigation_sequence: u64) -> Option<Ticket> {
        if self.paused || self.closed { return None; }
        self.next = self.next.checked_add(1)?;
        let ticket = Ticket { id: self.next, epoch: self.epoch, document_generation, navigation_sequence };
        self.candidates.insert(ticket.id, Candidate { ticket, child });
        Some(ticket)
    }
    pub fn admit<R>(&mut self, ticket: Ticket, document_generation: u64, navigation_sequence: u64, commit: impl FnOnce() -> R) -> Option<R> {
        if self.paused || self.closed || ticket.epoch != self.epoch
            || ticket.document_generation != document_generation || ticket.navigation_sequence != navigation_sequence
            || !self.candidates.get(&ticket.id).is_some_and(|candidate| candidate.ticket == ticket) { return None; }
        // The caller holds this owner's lock through the actual host commit.
        // Pause therefore orders either before or after tab registration.
        let result = commit();
        self.candidates.remove(&ticket.id);
        Some(result)
    }
    pub fn pause(&mut self) {
        self.paused = true;
        // A resumed owner never accepts a pre-Pause ticket, including one
        // already claimed by a worker waiting on another asynchronous phase.
        self.epoch = self.epoch.checked_add(1).unwrap_or_else(|| { self.closed = true; self.epoch });
    }
    pub fn resume(&mut self) { self.paused = false; }
    pub fn close(&mut self) { self.closed = true; self.pause(); }
    pub fn remove(&mut self, ticket: Ticket) { self.candidates.remove(&ticket.id); }
    pub fn children(&self) -> impl Iterator<Item = &T> { self.candidates.values().map(|candidate| &candidate.child) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn queued_and_claimed_tickets_are_revoked_and_resume_cannot_revive_them() {
        let mut admission = Admission::default();
        let queued = admission.reserve("queued-child", 4, 8).unwrap();
        let claimed = admission.reserve("claimed-child", 4, 8).unwrap();
        admission.pause();
        assert_eq!(admission.children().copied().collect::<Vec<_>>(), ["queued-child", "claimed-child"]);
        assert!(admission.reserve("paused-child", 4, 8).is_none());
        admission.resume();
        for ticket in [queued, claimed] {
            assert!(admission.admit(ticket, 4, 8, || panic!("revoked candidate committed")).is_none());
        }
        // Revocation keeps close ownership until the exact native ACK owner
        // removes the child, including when an earlier close attempt failed.
        assert_eq!(admission.children().count(), 2);
        admission.remove(queued);
        admission.remove(claimed);
        let fresh = admission.reserve("new-child", 4, 8).unwrap();
        assert_eq!(admission.admit(fresh, 4, 8, || "registered"), Some("registered"));
        assert_eq!(admission.children().count(), 0);
    }
    #[test]
    fn admission_requires_the_captured_document_and_navigation() {
        let mut admission = Admission::default();
        let ticket = admission.reserve("child", 4, 8).unwrap();
        assert!(admission.admit(ticket, 5, 8, || ()).is_none());
        assert!(admission.admit(ticket, 4, 9, || ()).is_none());
        assert_eq!(admission.children().count(), 1);
        assert!(admission.admit(ticket, 4, 8, || ()).is_some());
        assert!(admission.admit(ticket, 4, 8, || panic!("duplicate commit")).is_none());
    }
    #[test]
    fn closed_opener_never_resumes_admission() {
        let mut admission = Admission::default();
        let ticket = admission.reserve("child", 1, 2).unwrap();
        admission.close(); admission.resume();
        assert!(admission.admit(ticket, 1, 2, || ()).is_none());
        assert!(admission.reserve("later-child", 1, 2).is_none());
        assert_eq!(admission.children().count(), 1);
    }
    #[test]
    fn a_claimed_worker_cannot_commit_after_pause_ack_and_resume() {
        use std::sync::{Arc, Mutex, mpsc};
        let mut admission = Admission::default();
        let ticket = admission.reserve("native-child", 3, 7).unwrap();
        let admission = Arc::new(Mutex::new(admission));
        let worker_owner = admission.clone();
        let (claimed_tx, claimed_rx) = mpsc::channel();
        let (continue_tx, continue_rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            claimed_tx.send(()).unwrap();
            continue_rx.recv().unwrap();
            worker_owner.lock().unwrap().admit(ticket, 3, 7, || "late activation")
        });
        claimed_rx.recv().unwrap();
        {
            let mut owner = admission.lock().unwrap();
            owner.pause();
            // This represents the caller returning only after its native
            // close owner has acknowledged this exact child.
            owner.remove(ticket);
            owner.resume();
        }
        continue_tx.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), None);
    }
}
