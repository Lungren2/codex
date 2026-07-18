PRAGMA foreign_keys = ON;

CREATE TABLE IF NOT EXISTS repositories (
  root TEXT PRIMARY KEY,
  base_ref_override TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS completion_events (
  thread_id TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  payload_json TEXT NOT NULL,
  received_at TEXT NOT NULL,
  artifact_id TEXT,
  PRIMARY KEY (thread_id, turn_id)
);

CREATE TABLE IF NOT EXISTS patch_sets (
  diff_sha256 TEXT PRIMARY KEY,
  blob_path TEXT NOT NULL,
  worktree_fingerprint TEXT NOT NULL,
  created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS artifacts (
  artifact_id TEXT PRIMARY KEY,
  thread_id TEXT NOT NULL,
  turn_id TEXT NOT NULL,
  revision INTEGER NOT NULL,
  repository_root TEXT NOT NULL,
  branch TEXT,
  status TEXT NOT NULL,
  created_at TEXT NOT NULL,
  artifact_json TEXT NOT NULL,
  UNIQUE(thread_id, turn_id),
  UNIQUE(thread_id, revision)
);

CREATE TABLE IF NOT EXISTS synthesis_jobs (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  artifact_id TEXT NOT NULL UNIQUE,
  status TEXT NOT NULL,
  attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (artifact_id) REFERENCES artifacts(artifact_id)
);

CREATE TABLE IF NOT EXISTS feedback (
  id INTEGER PRIMARY KEY AUTOINCREMENT,
  artifact_id TEXT NOT NULL,
  revision INTEGER NOT NULL,
  diff_sha256 TEXT NOT NULL,
  status TEXT NOT NULL,
  feedback_json TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  FOREIGN KEY (artifact_id) REFERENCES artifacts(artifact_id)
);

CREATE TABLE IF NOT EXISTS reconciliation_cursors (
  source TEXT PRIMARY KEY,
  cursor TEXT,
  updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS artifacts_created_at_idx ON artifacts(created_at DESC);
CREATE INDEX IF NOT EXISTS feedback_artifact_idx ON feedback(artifact_id, created_at);

