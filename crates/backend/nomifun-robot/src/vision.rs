//! Bounded, text-only observations produced by the real robot vision endpoint.
//!
//! JPEG bytes remain request-local. Only the latest model answer is retained so
//! `robot.vision` can contribute actual device context without storing camera
//! images or introducing a second vision-model path.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

pub const VISION_OBSERVATION_MAX_AGE_MS: i64 = 5 * 60 * 1_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RobotVisionObservation {
    pub robot_id: String,
    pub companion_id: String,
    pub question: String,
    pub answer: String,
    pub observed_at_ms: i64,
}

#[derive(Default)]
pub struct RobotVisionObservationRegistry {
    inner: RwLock<BTreeMap<String, RobotVisionObservation>>,
}

impl RobotVisionObservationRegistry {
    pub async fn record(&self, observation: RobotVisionObservation) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn observation(companion_id: &str, observed_at_ms: i64) -> RobotVisionObservation {
        RobotVisionObservation {
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
