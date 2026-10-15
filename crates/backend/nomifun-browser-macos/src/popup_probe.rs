//! Debug-only deterministic gates for the real native popup admission path.
//! They do not create a popup, grant popup permission, or bypass its owner.
use super::{Page, PopupCandidate};
use std::{collections::BTreeMap, sync::{Arc, Mutex, OnceLock, Weak}};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PopupAdmissionStage { Queued, Claimed }
#[derive(Clone)]
pub struct PopupAdmissionObservation {
    pub candidate_id: u64,
    pub opener_id: Uuid,
    pub opener_document_generation: u64,
    pub opener_navigation_sequence: u64,
    pub child: Arc<Page>,
}
#[derive(Clone, Debug)]
pub struct PopupAdmissionSettlement {
    pub candidate_id: u64,
    pub child_id: Uuid,
    pub admitted: bool,
    pub native_closed: bool,
}
struct Gate {
    stage: PopupAdmissionStage,
    observed: watch::Sender<Option<PopupAdmissionObservation>>,
    settled: watch::Sender<Option<PopupAdmissionSettlement>>,
    release: CancellationToken,
}
fn gates() -> &'static Mutex<BTreeMap<Uuid, Weak<Gate>>> {
    static GATES: OnceLock<Mutex<BTreeMap<Uuid, Weak<Gate>>>> = OnceLock::new();
    GATES.get_or_init(Default::default)
}
pub struct PopupAdmissionHold { opener_id: Uuid, gate: Arc<Gate> }
pub fn hold_popup_admission(opener_id: Uuid, stage: PopupAdmissionStage) -> Result<PopupAdmissionHold, String> {
    let mut gates = gates().lock().unwrap();
    if gates.get(&opener_id).and_then(Weak::upgrade).is_some() { return Err("WK opener already has a popup admission gate".into()); }
    let (observed, _) = watch::channel(None);
    let (settled, _) = watch::channel(None);
    let gate = Arc::new(Gate { stage, observed, settled, release: CancellationToken::new() });
    gates.insert(opener_id, Arc::downgrade(&gate));
    Ok(PopupAdmissionHold { opener_id, gate })
}
impl PopupAdmissionHold {
    pub async fn wait_candidate(&self) -> Result<PopupAdmissionObservation, String> {
        let mut observed = self.gate.observed.subscribe();
        loop {
            if let Some(candidate) = observed.borrow_and_update().clone() { return Ok(candidate); }
            tokio::select! {
                _ = self.gate.release.cancelled() => return Err("WK popup gate released before candidate".into()),
                result = observed.changed() => result.map_err(|_| "WK popup gate observation lost".to_owned())?,
            }
        }
    }
    pub fn release(&self) { self.gate.release.cancel(); }
    pub async fn wait_settled(&self) -> Result<PopupAdmissionSettlement, String> {
        let mut settled = self.gate.settled.subscribe();
        loop {
            if let Some(receipt) = settled.borrow_and_update().clone() { return Ok(receipt); }
            settled.changed().await.map_err(|_| "WK popup settlement observation lost".to_owned())?;
        }
    }
}
impl Drop for PopupAdmissionHold {
    fn drop(&mut self) {
        self.release();
        let mut gates = gates().lock().unwrap();
        if gates.get(&self.opener_id).and_then(Weak::upgrade).is_some_and(|gate| Arc::ptr_eq(&gate, &self.gate)) {
            gates.remove(&self.opener_id);
        }
    }
}
pub(super) async fn wait_at_stage(candidate: &PopupCandidate, stage: PopupAdmissionStage) {
    let Some(opener) = candidate.opener.upgrade() else { return; };
    let gate = gates().lock().unwrap().get(&opener.id()).and_then(Weak::upgrade);
    let Some(gate) = gate.filter(|gate| gate.stage == stage && !gate.release.is_cancelled()) else { return; };
    // Only the first exact candidate is held and reported by this gate.
    let first = gate.observed.send_if_modified(|observation| {
        if observation.is_some() { return false; }
        *observation = Some(PopupAdmissionObservation {
            candidate_id: candidate.ticket.id, opener_id: opener.id(),
            opener_document_generation: candidate.ticket.document_generation,
            opener_navigation_sequence: candidate.ticket.navigation_sequence,
            child: candidate.page.clone(),
        });
        true
    });
    if first { gate.release.cancelled().await; }
}
pub(super) fn settlement_callback(candidate: &PopupCandidate) -> Box<dyn FnOnce(bool) + Send> {
    let gate = candidate.opener.upgrade().and_then(|opener|
        gates().lock().unwrap().get(&opener.id()).and_then(Weak::upgrade));
    let candidate_id = candidate.ticket.id;
    let child = candidate.page.clone();
    Box::new(move |admitted| {
        if let Some(gate) = gate
            && gate.observed.borrow().as_ref().is_some_and(|observed| observed.candidate_id == candidate_id) {
            gate.settled.send_replace(Some(PopupAdmissionSettlement {
                candidate_id, child_id: child.id(), admitted, native_closed: *child.closed().borrow(),
            }));
        }
    })
}
