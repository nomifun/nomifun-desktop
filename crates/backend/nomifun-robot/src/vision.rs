//! Bounded, text-only observations produced by the real robot vision endpoint.
//!
//! JPEG bytes remain request-local. Only the latest model answer is retained so
//! `robot.vision` can contribute actual device context without storing camera
//! images or introducing a second vision-model path.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, Weak};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

pub const VISION_OBSERVATION_MAX_AGE_MS: i64 = 5 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotVisionSource {
    pub companion_id: String,
    pub conversation_id: String,
    pub connection_id: String,
    pub request_id: String,
}

#[async_trait::async_trait]
pub trait RobotVisionRecorder: Send + Sync {
    fn robot_id(&self) -> &str;
    fn source(&self) -> RobotVisionSource;
    async fn authorize(&self) -> Result<(), String>;
    async fn record(&self, question: &str, answer: &str, jpeg: &[u8]) -> Result<(), String>;
}

type ActiveVisionTurns = Mutex<BTreeMap<String, Arc<dyn RobotVisionRecorder>>>;

pub struct RobotVisionTurnLease {
    active: Weak<ActiveVisionTurns>,
    robot_id: String,
    recorder: Arc<dyn RobotVisionRecorder>,
}

impl Drop for RobotVisionTurnLease {
    fn drop(&mut self) {
        if let Some(active) = self.active.upgrade() {
            let mut turns = active.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            if turns.get(&self.robot_id).is_some_and(|turn| Arc::ptr_eq(turn, &self.recorder)) {
                turns.remove(&self.robot_id);
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotVisionObservation {
    pub source: RobotVisionSource,
    pub robot_id: String,
    pub companion_id: String,
    pub question: String,
    pub answer: String,
    pub observed_at_ms: i64,
}

#[derive(Default)]
pub struct RobotVisionObservationRegistry {
    inner: RwLock<BTreeMap<String, RobotVisionObservation>>,
    active: Arc<ActiveVisionTurns>,
}

impl RobotVisionObservationRegistry {
    pub fn register_turn(&self, recorder: Arc<dyn RobotVisionRecorder>) -> RobotVisionTurnLease {
        let lease = RobotVisionTurnLease {
            active: Arc::downgrade(&self.active), robot_id: recorder.robot_id().to_owned(), recorder: recorder.clone(),
        };
        self.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .insert(lease.robot_id.clone(), recorder);
        lease
    }

    pub fn active_turn(&self, robot_id: &str, companion_id: &str) -> Option<Arc<dyn RobotVisionRecorder>> {
        self.active.lock().unwrap_or_else(std::sync::PoisonError::into_inner)
            .get(robot_id).filter(|turn| turn.source().companion_id == companion_id).cloned()
    }

    pub async fn record(&self, mut observation: RobotVisionObservation) {
        // Bound the retained source, not just its eventual prompt projection.
        // Visible markers prevent a partial observation being read as complete.
        truncate_observation(&mut observation.question, 1024);
        truncate_observation(&mut observation.answer, 8192);
        self.inner
            .write()
            .await
            .insert(observation.robot_id.clone(), observation);
    }

    /// Return context only for the robot's current Companion and only while it
    /// can honestly be described as a recent observation.
    pub async fn latest_recent(
        &self,
        robot_id: &str,
        companion_id: &str,
        now_ms: i64,
    ) -> Option<RobotVisionObservation> {
        let observation = self.inner.read().await.get(robot_id).cloned()?;
        let age_ms = now_ms.checked_sub(observation.observed_at_ms)?;
        (observation.companion_id == companion_id
            && (0..=VISION_OBSERVATION_MAX_AGE_MS).contains(&age_ms))
        .then_some(observation)
    }

    pub async fn remove(&self, robot_id: &str) {
        self.inner.write().await.remove(robot_id);
    }
}

fn truncate_observation(text: &mut String, max_bytes: usize) {
    if text.len() <= max_bytes {
        return;
    }
    const MARKER: &str = " [observation truncated]";
    let mut end = max_bytes.saturating_sub(MARKER.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str(MARKER);
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Recorder;
    #[async_trait::async_trait]
    impl RobotVisionRecorder for Recorder {
        fn robot_id(&self) -> &str { "robot-1" }
        fn source(&self) -> RobotVisionSource { observation("companion-1", 1).source }
        async fn authorize(&self) -> Result<(), String> { Ok(()) }
        async fn record(&self, _: &str, _: &str, _: &[u8]) -> Result<(), String> { Ok(()) }
    }

    #[test]
    fn an_old_lease_cannot_remove_a_replacement_with_the_same_source_ids() {
        let registry = RobotVisionObservationRegistry::default();
        let old = registry.register_turn(Arc::new(Recorder));
        let current = registry.register_turn(Arc::new(Recorder));
        drop(old);
        assert!(registry.active_turn("robot-1", "companion-1").is_some());
        assert!(registry.active_turn("robot-1", "other").is_none());
        drop(current);
        assert!(registry.active_turn("robot-1", "companion-1").is_none());
    }

    fn observation(companion_id: &str, observed_at_ms: i64) -> RobotVisionObservation {
        RobotVisionObservation {
            source: RobotVisionSource { companion_id: companion_id.to_owned(),
                conversation_id: "conversation-1".to_owned(), connection_id: "socket-1".to_owned(), request_id: "turn-1".to_owned() },
            robot_id: "robot-1".to_owned(),
            companion_id: companion_id.to_owned(),
            question: "桌上是什么？".to_owned(),
            answer: "桌上有一杯咖啡。".to_owned(),
            observed_at_ms,
        }
    }

    #[tokio::test]
    async fn observations_are_binding_scoped_and_expire() {
        let registry = RobotVisionObservationRegistry::default();
        registry.record(observation("companion-1", 1_000)).await;
        assert!(
            registry
                .latest_recent("robot-1", "companion-1", 1_001)
                .await
                .is_some()
        );
        assert!(
            registry
                .latest_recent("robot-1", "companion-2", 1_001)
                .await
                .is_none()
        );
        assert!(
            registry
                .latest_recent(
                    "robot-1",
                    "companion-1",
                    1_000 + VISION_OBSERVATION_MAX_AGE_MS + 1,
                )
                .await
                .is_none()
        );
    }
}
