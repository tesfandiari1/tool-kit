use std::{collections::HashMap, sync::Arc};

use tokio::sync::RwLock;
use uuid::Uuid;

use super::model::{JobRecord, JobView};

#[derive(Clone, Debug)]
pub struct JobRegistry {
    inner: Arc<RwLock<RegistryInner>>,
    max_jobs: usize,
}

impl JobRegistry {
    pub fn new(max_jobs: usize) -> Self {
        Self {
            inner: Arc::new(RwLock::new(RegistryInner::default())),
            max_jobs,
        }
    }

    pub async fn register(
        &self,
        idempotency_key: String,
        fingerprint: String,
        job: JobRecord,
    ) -> IdempotencyDecision {
        let mut inner = self.inner.write().await;
        if let Some(existing) = inner.idempotency.get(&idempotency_key) {
            if existing.fingerprint != fingerprint {
                return IdempotencyDecision::Conflict;
            }
            let job = inner
                .jobs
                .get(&existing.job_id)
                .expect("idempotency records must point to an existing job");
            return IdempotencyDecision::Replay(job.view());
        }
        if inner.jobs.len() >= self.max_jobs {
            return IdempotencyDecision::Capacity;
        }

        let view = job.view();
        inner.idempotency.insert(
            idempotency_key,
            IdempotencyRecord {
                fingerprint,
                job_id: job.id,
            },
        );
        inner.jobs.insert(job.id, job);
        IdempotencyDecision::New(view)
    }

    pub async fn get(&self, job_id: Uuid) -> Option<JobRecord> {
        self.inner.read().await.jobs.get(&job_id).cloned()
    }

    pub async fn accepting_jobs(&self) -> bool {
        self.inner.read().await.jobs.len() < self.max_jobs
    }

    pub async fn update(&self, job_id: Uuid, update: impl FnOnce(&mut JobRecord)) -> bool {
        let mut inner = self.inner.write().await;
        let Some(job) = inner.jobs.get_mut(&job_id) else {
            return false;
        };
        update(job);
        job.touch();
        true
    }
}

#[derive(Clone, Debug)]
pub enum IdempotencyDecision {
    New(JobView),
    Replay(JobView),
    Conflict,
    Capacity,
}

#[derive(Debug, Default)]
struct RegistryInner {
    jobs: HashMap<Uuid, JobRecord>,
    idempotency: HashMap<String, IdempotencyRecord>,
}

#[derive(Debug)]
struct IdempotencyRecord {
    fingerprint: String,
    job_id: Uuid,
}

#[cfg(test)]
mod tests {
    use uuid::Uuid;

    use crate::{
        artifacts::AttemptPaths,
        conversion::model::{ConversionProfile, JobRecord, SourceMetadata},
    };

    use super::{IdempotencyDecision, JobRegistry};

    #[tokio::test]
    async fn idempotency_replays_only_the_same_fingerprint() {
        let registry = JobRegistry::new(1);
        let job = job_record();
        let id = job.id;

        assert!(matches!(
            registry
                .register("key".to_owned(), "fingerprint".to_owned(), job)
                .await,
            IdempotencyDecision::New(_)
        ));
        let replay = registry
            .register("key".to_owned(), "fingerprint".to_owned(), job_record())
            .await;
        assert!(matches!(replay, IdempotencyDecision::Replay(view) if view.id == id));
        assert!(matches!(
            registry
                .register("key".to_owned(), "different".to_owned(), job_record())
                .await,
            IdempotencyDecision::Conflict
        ));
    }

    fn job_record() -> JobRecord {
        let attempt = std::env::temp_dir().join(Uuid::new_v4().to_string());
        JobRecord::new(
            Uuid::new_v4(),
            Uuid::new_v4(),
            Uuid::new_v4(),
            ConversionProfile::Standard,
            SourceMetadata {
                byte_length: 1,
                sha256: "0".repeat(64),
            },
            AttemptPaths {
                source: attempt.join("source.pdf"),
                publication_staging: attempt.join("publication.staging"),
                published: attempt.join("artifacts"),
                attempt,
            },
            Uuid::new_v4().to_string(),
        )
    }
}
