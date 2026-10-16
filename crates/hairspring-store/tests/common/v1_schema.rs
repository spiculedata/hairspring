//! The version 1 schemas exactly as the first release created them, so tests can build a genuine old database and
//! check that it migrates in place.
#![allow(dead_code)]

pub const SQLITE_V1: &str = "
CREATE TABLE runs (
  id TEXT PRIMARY KEY, target TEXT NOT NULL, backend TEXT NOT NULL, state TEXT NOT NULL,
  seed INTEGER NOT NULL, config_json TEXT NOT NULL, created_at INTEGER NOT NULL,
  started_at INTEGER, finished_at INTEGER, steps INTEGER NOT NULL DEFAULT 0,
  trials INTEGER NOT NULL DEFAULT 0, error TEXT);
CREATE INDEX runs_created ON runs(created_at);
CREATE TABLE failures (
  id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), target TEXT NOT NULL, message TEXT NOT NULL,
  original_len INTEGER NOT NULL, shrunk_len INTEGER NOT NULL, bundle_hash TEXT NOT NULL,
  steps_json TEXT NOT NULL, created_at INTEGER NOT NULL);
CREATE INDEX failures_run ON failures(run_id);
CREATE TABLE blobs (hash TEXT PRIMARY KEY, data BLOB NOT NULL);
CREATE TABLE jobs (
  id TEXT PRIMARY KEY, run_id TEXT NOT NULL REFERENCES runs(id), kind TEXT NOT NULL, payload_json TEXT NOT NULL,
  status TEXT NOT NULL, lease_owner TEXT, lease_until INTEGER, attempts INTEGER NOT NULL DEFAULT 0,
  last_error TEXT, created_at INTEGER NOT NULL);
CREATE INDEX jobs_runnable ON jobs(status, created_at);
CREATE TABLE events (
  id INTEGER PRIMARY KEY AUTOINCREMENT, run_id TEXT NOT NULL, ts INTEGER NOT NULL,
  kind TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE INDEX events_run ON events(run_id, id);
CREATE TABLE workers (
  id TEXT PRIMARY KEY, name TEXT NOT NULL, caps_json TEXT NOT NULL,
  registered_at INTEGER NOT NULL, last_seen INTEGER NOT NULL);
";

pub const POSTGRES_V1: &str = "
CREATE TABLE runs (
  id TEXT PRIMARY KEY, seq BIGSERIAL, target TEXT NOT NULL, backend TEXT NOT NULL, state TEXT NOT NULL,
  seed BIGINT NOT NULL, config_json TEXT NOT NULL, created_at BIGINT NOT NULL,
  started_at BIGINT, finished_at BIGINT, steps BIGINT NOT NULL DEFAULT 0, trials BIGINT NOT NULL DEFAULT 0, error TEXT);
CREATE INDEX runs_created ON runs(created_at, seq);
CREATE TABLE failures (
  id TEXT PRIMARY KEY, seq BIGSERIAL, run_id TEXT NOT NULL REFERENCES runs(id), target TEXT NOT NULL, message TEXT NOT NULL,
  original_len BIGINT NOT NULL, shrunk_len BIGINT NOT NULL, bundle_hash TEXT NOT NULL, steps_json TEXT NOT NULL, created_at BIGINT NOT NULL);
CREATE INDEX failures_run ON failures(run_id);
CREATE TABLE blobs (hash TEXT PRIMARY KEY, data BYTEA NOT NULL);
CREATE TABLE jobs (
  id TEXT PRIMARY KEY, seq BIGSERIAL, run_id TEXT NOT NULL REFERENCES runs(id), kind TEXT NOT NULL, payload_json TEXT NOT NULL,
  status TEXT NOT NULL, lease_owner TEXT, lease_until BIGINT, attempts INTEGER NOT NULL DEFAULT 0, last_error TEXT, created_at BIGINT NOT NULL);
CREATE INDEX jobs_runnable ON jobs(status, created_at, seq);
CREATE TABLE events (
  id BIGSERIAL PRIMARY KEY, run_id TEXT NOT NULL, ts BIGINT NOT NULL, kind TEXT NOT NULL, payload_json TEXT NOT NULL);
CREATE INDEX events_run ON events(run_id, id);
CREATE TABLE workers (
  id TEXT PRIMARY KEY, seq BIGSERIAL, name TEXT NOT NULL, caps_json TEXT NOT NULL, registered_at BIGINT NOT NULL, last_seen BIGINT NOT NULL);
";
