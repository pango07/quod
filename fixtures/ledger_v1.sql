-- A ledger as the v1 binary left it (schema version 1 with rows), for the
-- migration test in quod-ledger. The migration to v2 must keep every row.
CREATE TABLE runs      (id TEXT PRIMARY KEY, started_at TEXT NOT NULL, commit_sha TEXT, host TEXT);
CREATE TABLE manifests (id TEXT PRIMARY KEY, step_key TEXT NOT NULL, run_id TEXT NOT NULL, created_at TEXT NOT NULL, json BLOB NOT NULL);
CREATE INDEX manifests_by_key ON manifests (step_key, created_at DESC);
CREATE TABLE results   (id TEXT PRIMARY KEY, step_key TEXT NOT NULL, manifest_id TEXT NOT NULL, run_id TEXT NOT NULL, created_at TEXT NOT NULL, json BLOB NOT NULL);
CREATE INDEX results_by_key ON results (step_key, manifest_id, created_at DESC);
CREATE TABLE blobs     (hash TEXT PRIMARY KEY, size INTEGER NOT NULL, path TEXT NOT NULL);
INSERT INTO runs VALUES ('01V1RUN0000000000000000000', '2026-10-30T12:00:00.000Z', '9490d204a2ba', 'box');
INSERT INTO manifests VALUES (
  'b3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  'b3:1111111111111111111111111111111111111111111111111111111111111111',
  '01V1RUN0000000000000000000', '2026-10-30T12:00:01.000Z',
  CAST('{"entries":[{"h":"b3:2222222222222222222222222222222222222222222222222222222222222222","k":"f","m":420,"p":"go.mod"}],"roots":["/workspace"],"step_key":"b3:1111111111111111111111111111111111111111111111111111111111111111","v":1}' AS BLOB)
);
INSERT INTO results VALUES (
  '01V1RUN0000000000000000000',
  'b3:1111111111111111111111111111111111111111111111111111111111111111',
  'b3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa',
  '01V1RUN0000000000000000000', '2026-10-30T12:00:02.000Z',
  CAST('{"duration_ms":1234,"exit_code":0,"manifest_id":"b3:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","outputs":[],"recorded_at":"2026-10-30T12:00:02.000Z","run_id":"01V1RUN0000000000000000000","stderr":"b3:3333333333333333333333333333333333333333333333333333333333333333","stdout":"b3:4444444444444444444444444444444444444444444444444444444444444444","step_key":"b3:1111111111111111111111111111111111111111111111111111111111111111","v":1}' AS BLOB)
);
INSERT INTO blobs VALUES ('b3:2222222222222222222222222222222222222222222222222222222222222222', 9, '22/2222');
