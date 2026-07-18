use crate::models::{
    ArtifactListItem, ArtifactStatus, DecisionStatus, FeedbackRecord, ReviewArtifact,
    ReviewFeedback,
};
use crate::synthesis::SynthesisRun;
use anyhow::{Context, Result};
use chrono::Utc;
use rusqlite::{Connection, OptionalExtension, TransactionBehavior, params};
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    connection: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let connection = Connection::open(path)?;
        connection.execute_batch("PRAGMA journal_mode = WAL; PRAGMA foreign_keys = ON;")?;
        connection.execute_batch(include_str!("../migrations/0001_initial.sql"))?;
        Ok(Self {
            connection: Mutex::new(connection),
        })
    }

    pub fn repository_base_override(&self, root: &str) -> Result<Option<String>> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        Ok(connection
            .query_row(
                "SELECT base_ref_override FROM repositories WHERE root = ?1",
                [root],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    pub fn set_repository_base_override(&self, root: &str, base_ref: &str) -> Result<()> {
        let now = Utc::now().to_rfc3339();
        let connection = self.connection.lock().expect("store mutex poisoned");
        connection.execute(
            "INSERT INTO repositories(root, base_ref_override, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?3)
             ON CONFLICT(root) DO UPDATE SET base_ref_override = excluded.base_ref_override,
               updated_at = excluded.updated_at",
            params![root, base_ref, now],
        )?;
        Ok(())
    }

    pub fn existing_completion(&self, thread_id: &str, turn_id: &str) -> Result<Option<String>> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        Ok(connection
            .query_row(
                "SELECT artifact_id FROM completion_events WHERE thread_id = ?1 AND turn_id = ?2",
                params![thread_id, turn_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten())
    }

    pub fn next_revision(&self, thread_id: &str) -> Result<u32> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let revision: i64 = connection.query_row(
            "SELECT COALESCE(MAX(revision), 0) + 1 FROM artifacts WHERE thread_id = ?1",
            [thread_id],
            |row| row.get(0),
        )?;
        Ok(revision as u32)
    }

    pub fn existing_fingerprint(
        &self,
        thread_id: &str,
        fingerprint: &str,
    ) -> Result<Option<String>> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT artifact_id, artifact_json FROM artifacts WHERE thread_id = ?1 ORDER BY revision DESC",
        )?;
        let rows = statement.query_map([thread_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        for row in rows {
            let (artifact_id, json) = row?;
            let artifact: ReviewArtifact = serde_json::from_str(&json)?;
            if artifact.repository.worktree_fingerprint == fingerprint {
                return Ok(Some(artifact_id));
            }
        }
        Ok(None)
    }

    pub fn link_completion(
        &self,
        thread_id: &str,
        turn_id: &str,
        payload_json: &str,
        artifact_id: &str,
    ) -> Result<()> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        connection.execute(
            "INSERT OR IGNORE INTO completion_events(thread_id, turn_id, payload_json, received_at, artifact_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![thread_id, turn_id, payload_json, Utc::now().to_rfc3339(), artifact_id],
        )?;
        Ok(())
    }

    pub fn persist_capture(&self, payload_json: &str, artifact: &ReviewArtifact) -> Result<String> {
        let now = Utc::now().to_rfc3339();
        let artifact_json = serde_json::to_string(artifact)?;
        let mut connection = self.connection.lock().expect("store mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = transaction
            .query_row(
                "SELECT artifact_id FROM completion_events WHERE thread_id = ?1 AND turn_id = ?2",
                params![artifact.thread_id, artifact.turn_id],
                |row| row.get(0),
            )
            .optional()?
            .flatten();
        if let Some(artifact_id) = existing {
            transaction.commit()?;
            return Ok(artifact_id);
        }
        transaction.execute(
            "INSERT INTO repositories(root, created_at, updated_at) VALUES (?1, ?2, ?2)
             ON CONFLICT(root) DO UPDATE SET updated_at = excluded.updated_at",
            params![artifact.repository.root, now],
        )?;
        transaction.execute(
            "INSERT OR IGNORE INTO patch_sets(diff_sha256, blob_path, worktree_fingerprint, created_at)
             VALUES (?1, ?2, ?3, ?4)",
            params![artifact.patch_set.diff_sha256, artifact.patch_set.blob_path, artifact.repository.worktree_fingerprint, now],
        )?;
        transaction.execute(
            "INSERT INTO artifacts(artifact_id, thread_id, turn_id, revision, repository_root, branch, status, created_at, artifact_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![artifact.artifact_id, artifact.thread_id, artifact.turn_id, artifact.revision,
                artifact.repository.root, artifact.repository.branch, status_name(&artifact.status), artifact.created_at, artifact_json],
        )?;
        transaction.execute(
            "INSERT INTO completion_events(thread_id, turn_id, payload_json, received_at, artifact_id)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![artifact.thread_id, artifact.turn_id, payload_json, now, artifact.artifact_id],
        )?;
        transaction.execute(
            "INSERT INTO synthesis_jobs(artifact_id, status, created_at, updated_at) VALUES (?1, 'queued', ?2, ?2)",
            params![artifact.artifact_id, now],
        )?;
        transaction.commit()?;
        Ok(artifact.artifact_id.clone())
    }

    pub fn list_artifacts(&self) -> Result<Vec<ArtifactListItem>> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let mut statement =
            connection.prepare("SELECT artifact_json FROM artifacts ORDER BY created_at DESC")?;
        let json_rows = statement.query_map([], |row| row.get::<_, String>(0))?;
        json_rows
            .map(|json| {
                let artifact: ReviewArtifact = serde_json::from_str(&json?)?;
                Ok(ArtifactListItem {
                    artifact_id: artifact.artifact_id,
                    thread_id: artifact.thread_id,
                    turn_id: artifact.turn_id,
                    revision: artifact.revision,
                    title: artifact.title,
                    summary: artifact.summary,
                    status: artifact.status,
                    created_at: artifact.created_at,
                    repository_root: artifact.repository.root,
                    branch: artifact.repository.branch,
                    decision_count: artifact.decisions.len(),
                    unresolved_count: artifact
                        .decisions
                        .iter()
                        .filter(|decision| decision.status == DecisionStatus::Unresolved)
                        .count(),
                })
            })
            .collect::<Result<Vec<_>>>()
    }

    pub fn get_artifact(&self, artifact_id: &str) -> Result<Option<ReviewArtifact>> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let json: Option<String> = connection
            .query_row(
                "SELECT artifact_json FROM artifacts WHERE artifact_id = ?1",
                [artifact_id],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).context("stored artifact is invalid"))
            .transpose()
    }

    pub fn queue_feedback(&self, feedback: &ReviewFeedback) -> Result<String> {
        let artifact = self
            .get_artifact(&feedback.artifact_id)?
            .context("review artifact no longer exists")?;
        anyhow::ensure!(
            artifact.revision == feedback.revision,
            "feedback revision is stale"
        );
        anyhow::ensure!(
            artifact.patch_set.diff_sha256 == feedback.diff_sha256,
            "feedback patch identity is stale"
        );
        anyhow::ensure!(
            !feedback.comment.trim().is_empty(),
            "feedback comment cannot be empty"
        );
        let now = Utc::now().to_rfc3339();
        let connection = self.connection.lock().expect("store mutex poisoned");
        let status = if feedback.stale { "orphaned" } else { "queued" };
        connection.execute(
            "INSERT INTO feedback(artifact_id, revision, diff_sha256, status, feedback_json, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?6)",
            params![feedback.artifact_id, feedback.revision, feedback.diff_sha256, status, serde_json::to_string(feedback)?, now],
        )?;
        Ok(connection.last_insert_rowid().to_string())
    }

    pub fn list_feedback(&self, artifact_id: &str) -> Result<Vec<FeedbackRecord>> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let mut statement = connection.prepare(
            "SELECT id, status, feedback_json, created_at, updated_at
             FROM feedback WHERE artifact_id = ?1 ORDER BY created_at DESC",
        )?;
        let rows = statement.query_map([artifact_id], |row| {
            Ok((
                row.get::<_, i64>(0)?.to_string(),
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, String>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (id, status, json, created_at, updated_at) = row?;
            Ok(FeedbackRecord {
                id,
                status,
                feedback: serde_json::from_str(&json)?,
                created_at,
                updated_at,
            })
        })
        .collect()
    }

    pub fn mark_latest_feedback_status(&self, thread_id: &str, status: &str) -> Result<()> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        connection.execute(
            "UPDATE feedback SET status = ?2, updated_at = ?3 WHERE id = (
                SELECT f.id FROM feedback f JOIN artifacts a ON a.artifact_id = f.artifact_id
                WHERE a.thread_id = ?1 AND f.status IN ('queued', 'running')
                ORDER BY f.created_at DESC LIMIT 1
             )",
            params![thread_id, status, Utc::now().to_rfc3339()],
        )?;
        Ok(())
    }

    pub fn feedback_handoff(&self, feedback_id: i64) -> Result<(String, ReviewFeedback)> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let (thread_id, feedback_json): (String, String) = connection.query_row(
            "SELECT a.thread_id, f.feedback_json
             FROM feedback f JOIN artifacts a ON a.artifact_id = f.artifact_id
             WHERE f.id = ?1",
            [feedback_id],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )?;
        Ok((thread_id, serde_json::from_str(&feedback_json)?))
    }

    pub fn claim_synthesis_job(&self) -> Result<Option<(ReviewArtifact, String)>> {
        let mut connection = self.connection.lock().expect("store mutex poisoned");
        let transaction = connection.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let job: Option<(String, String, String)> = transaction
            .query_row(
                "SELECT j.artifact_id, a.artifact_json, e.payload_json
                 FROM synthesis_jobs j
                 JOIN artifacts a ON a.artifact_id = j.artifact_id
                 JOIN completion_events e ON e.artifact_id = j.artifact_id
                 WHERE j.status = 'queued' ORDER BY j.created_at LIMIT 1",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .optional()?;
        let Some((artifact_id, artifact_json, payload_json)) = job else {
            transaction.commit()?;
            return Ok(None);
        };
        transaction.execute(
            "UPDATE synthesis_jobs SET status = 'running', attempts = attempts + 1, updated_at = ?2 WHERE artifact_id = ?1",
            params![artifact_id, Utc::now().to_rfc3339()],
        )?;
        transaction.execute(
            "UPDATE artifacts SET status = 'synthesizing' WHERE artifact_id = ?1",
            [artifact_id],
        )?;
        transaction.commit()?;
        Ok(Some((serde_json::from_str(&artifact_json)?, payload_json)))
    }

    pub fn finish_synthesis(&self, run: &SynthesisRun) -> Result<()> {
        let mut artifact = run.artifact.clone();
        if artifact.status == ArtifactStatus::Ready {
            let previous = self.previous_artifact(&artifact.thread_id, artifact.revision)?;
            artifact.revision_comparison = compare_revisions(previous.as_ref(), &artifact);
        }
        let status = status_name(&artifact.status);
        let job_status = if run.error.is_some() {
            "failed"
        } else {
            "completed"
        };
        let now = Utc::now().to_rfc3339();
        let connection = self.connection.lock().expect("store mutex poisoned");
        connection.execute(
            "UPDATE artifacts SET status = ?2, artifact_json = ?3 WHERE artifact_id = ?1",
            params![
                artifact.artifact_id,
                status,
                serde_json::to_string(&artifact)?
            ],
        )?;
        connection.execute(
            "UPDATE synthesis_jobs SET status = ?2, last_error = ?3, updated_at = ?4 WHERE artifact_id = ?1",
            params![artifact.artifact_id, job_status, run.error, now],
        )?;
        if artifact.status == ArtifactStatus::Ready {
            supersede_older(&connection, &artifact.thread_id, artifact.revision)?;
            drop(connection);
            self.mark_latest_feedback_status(&artifact.thread_id, "addressed")?;
        } else if run.error.is_some() {
            drop(connection);
            self.mark_latest_feedback_status(&artifact.thread_id, "failedRetryable")?;
        }
        Ok(())
    }

    fn previous_artifact(&self, thread_id: &str, revision: u32) -> Result<Option<ReviewArtifact>> {
        let connection = self.connection.lock().expect("store mutex poisoned");
        let json: Option<String> = connection
            .query_row(
                "SELECT artifact_json FROM artifacts
                 WHERE thread_id = ?1 AND revision < ?2
                 ORDER BY revision DESC LIMIT 1",
                params![thread_id, revision],
                |row| row.get(0),
            )
            .optional()?;
        json.map(|json| serde_json::from_str(&json).context("prior artifact is invalid"))
            .transpose()
    }

    pub fn defer_synthesis(&self, artifact_id: &str, error: &str) -> Result<()> {
        let mut artifact = self
            .get_artifact(artifact_id)?
            .context("artifact does not exist")?;
        let now = Utc::now().to_rfc3339();
        let connection = self.connection.lock().expect("store mutex poisoned");
        let attempts: i64 = connection.query_row(
            "SELECT attempts FROM synthesis_jobs WHERE artifact_id = ?1",
            [artifact_id],
            |row| row.get(0),
        )?;
        if attempts >= 3 {
            artifact.status = ArtifactStatus::Partial;
            connection.execute(
                "UPDATE artifacts SET status = 'partial', artifact_json = ?2 WHERE artifact_id = ?1",
                params![artifact_id, serde_json::to_string(&artifact)?],
            )?;
            connection.execute(
                "UPDATE synthesis_jobs SET status = 'failed', last_error = ?2, updated_at = ?3 WHERE artifact_id = ?1",
                params![artifact_id, error, now],
            )?;
        } else {
            connection.execute(
                "UPDATE synthesis_jobs SET status = 'queued', last_error = ?2, updated_at = ?3 WHERE artifact_id = ?1",
                params![artifact_id, error, now],
            )?;
        }
        Ok(())
    }
}

fn compare_revisions(
    previous: Option<&ReviewArtifact>,
    current: &ReviewArtifact,
) -> crate::models::RevisionComparison {
    let Some(previous) = previous else {
        return crate::models::RevisionComparison {
            new_decision_ids: current
                .decisions
                .iter()
                .map(|decision| decision.id.clone())
                .collect(),
            ..Default::default()
        };
    };
    let mut matched_previous = std::collections::HashSet::new();
    let mut unchanged = Vec::new();
    let mut addressed = Vec::new();
    let mut introduced = Vec::new();
    for decision in &current.decisions {
        let current_evidence = evidence_keys(decision);
        let exact = previous
            .decisions
            .iter()
            .find(|prior| prior.id == decision.id);
        let overlap = previous.decisions.iter().find(|prior| {
            let prior_evidence = evidence_keys(prior);
            !current_evidence.is_disjoint(&prior_evidence)
        });
        match exact.or(overlap) {
            Some(prior) => {
                matched_previous.insert(prior.id.clone());
                if evidence_keys(prior) == current_evidence && prior.summary == decision.summary {
                    unchanged.push(decision.id.clone());
                } else {
                    addressed.push(decision.id.clone());
                }
            }
            None => introduced.push(decision.id.clone()),
        }
    }
    let invalidated = previous
        .decisions
        .iter()
        .filter(|decision| !matched_previous.contains(&decision.id))
        .map(|decision| decision.id.clone())
        .collect();
    crate::models::RevisionComparison {
        previous_revision: Some(previous.revision),
        unchanged_decision_ids: unchanged,
        addressed_decision_ids: addressed,
        new_decision_ids: introduced,
        invalidated_decision_ids: invalidated,
    }
}

fn evidence_keys(
    decision: &crate::models::ReviewDecision,
) -> std::collections::HashSet<(String, String)> {
    decision
        .evidence
        .iter()
        .map(|evidence| (evidence.path.clone(), evidence.hunk_id.clone()))
        .collect()
}

fn supersede_older(connection: &Connection, thread_id: &str, revision: u32) -> Result<()> {
    let rows = {
        let mut statement = connection.prepare(
            "SELECT artifact_id, artifact_json FROM artifacts
             WHERE thread_id = ?1 AND revision < ?2 AND status != 'superseded'",
        )?;
        statement
            .query_map(params![thread_id, revision], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?
    };
    for (artifact_id, json) in rows {
        let mut artifact: ReviewArtifact = serde_json::from_str(&json)?;
        artifact.status = ArtifactStatus::Superseded;
        connection.execute(
            "UPDATE artifacts SET status = 'superseded', artifact_json = ?2 WHERE artifact_id = ?1",
            params![artifact_id, serde_json::to_string(&artifact)?],
        )?;
    }
    Ok(())
}

fn status_name(status: &ArtifactStatus) -> &'static str {
    match status {
        ArtifactStatus::Collecting => "collecting",
        ArtifactStatus::Synthesizing => "synthesizing",
        ArtifactStatus::Ready => "ready",
        ArtifactStatus::Partial => "partial",
        ArtifactStatus::Stale => "stale",
        ArtifactStatus::Superseded => "superseded",
        ArtifactStatus::Unavailable => "unavailable",
    }
}

#[cfg(test)]
#[path = "store_tests.rs"]
mod tests;
