CREATE TABLE conversions (
    id                    TEXT PRIMARY KEY NOT NULL
                                  CHECK(length(id) = 36 AND id = lower(id)),
    client_run_id         TEXT NOT NULL
                                  CHECK(length(client_run_id) = 36
                                    AND client_run_id = lower(client_run_id)),
    auth_scope            TEXT NOT NULL
                                  CHECK(auth_scope = 'bootstrap'),
    idempotency_key_hash  TEXT NOT NULL
                                  CHECK(length(idempotency_key_hash) = 64
                                    AND idempotency_key_hash
                                        NOT GLOB '*[^0-9a-f]*'),
    request_fingerprint   TEXT NOT NULL
                                  CHECK(length(request_fingerprint) = 64
                                    AND request_fingerprint
                                        NOT GLOB '*[^0-9a-f]*'),
    profile               TEXT NOT NULL
                                  CHECK(profile IN (
                                      'standard',
                                      'local_only',
                                      'best_quality'
                                  )),
    status                TEXT NOT NULL
                                  CHECK(status IN (
                                      'queued',
                                      'converting_local',
                                      'finalizing',
                                      'succeeded',
                                      'failed',
                                      'needs_remote'
                                  )),
    source_relative_path  TEXT NOT NULL,
    source_media_type     TEXT NOT NULL
                                  CHECK(source_media_type = 'application/pdf'),
    source_byte_length    INTEGER NOT NULL
                                  CHECK(source_byte_length > 0),
    source_sha256         TEXT NOT NULL
                                  CHECK(length(source_sha256) = 64
                                    AND source_sha256
                                        NOT GLOB '*[^0-9a-f]*'),
    active_attempt_id     TEXT,
    route                 TEXT,
    reason_codes_json     TEXT NOT NULL DEFAULT '[]'
                                  CHECK(length(reason_codes_json) <= 8192
                                    AND json_valid(reason_codes_json)
                                    AND json_type(reason_codes_json) = 'array'),
    warnings_json         TEXT NOT NULL DEFAULT '[]'
                                  CHECK(length(warnings_json) <= 65536
                                    AND json_valid(warnings_json)
                                    AND json_type(warnings_json) = 'array'),
    failure_code          TEXT,
    failure_message       TEXT,
    origin_request_id     TEXT NOT NULL,
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL,

    UNIQUE(auth_scope, idempotency_key_hash),

    CHECK(length(source_relative_path) <= 512
      AND source_relative_path = 'jobs/' || id || '/source/input'),
    CHECK(length(origin_request_id) BETWEEN 1 AND 128),
    CHECK(route IS NULL OR length(route) BETWEEN 1 AND 128),
    CHECK(failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 128),
    CHECK(failure_message IS NULL OR length(failure_message) BETWEEN 1 AND 1024),
    CHECK((failure_code IS NULL) = (failure_message IS NULL)),

    FOREIGN KEY(id, active_attempt_id)
        REFERENCES attempts(conversion_id, id)
        DEFERRABLE INITIALLY DEFERRED
) STRICT;

CREATE TABLE attempts (
    queue_seq             INTEGER PRIMARY KEY AUTOINCREMENT,
    id                    TEXT NOT NULL UNIQUE
                                  CHECK(length(id) = 36 AND id = lower(id)),
    conversion_id         TEXT NOT NULL,
    attempt_number        INTEGER NOT NULL
                                  CHECK(attempt_number >= 1),
    state                 TEXT NOT NULL
                                  CHECK(state IN (
                                      'queued',
                                      'converting_local',
                                      'finalizing',
                                      'succeeded',
                                      'failed',
                                      'needs_remote',
                                      'interrupted'
                                  )),
    recovery_count        INTEGER NOT NULL DEFAULT 0
                                  CHECK(recovery_count >= 0),
    engine_name           TEXT,
    engine_version        TEXT,
    route                 TEXT,
    classification        TEXT
                                  CHECK(classification IS NULL OR
                                        classification IN (
                                            'text_based',
                                            'scanned',
                                            'image_based',
                                            'mixed'
                                        )),
    inspection_json       TEXT
                                  CHECK(inspection_json IS NULL OR (
                                      length(inspection_json) <= 1048576
                                      AND json_valid(inspection_json)
                                      AND json_type(inspection_json) = 'object'
                                  )),
    reason_codes_json     TEXT NOT NULL DEFAULT '[]'
                                  CHECK(length(reason_codes_json) <= 8192
                                    AND json_valid(reason_codes_json)
                                    AND json_type(reason_codes_json) = 'array'),
    warnings_json         TEXT NOT NULL DEFAULT '[]'
                                  CHECK(length(warnings_json) <= 65536
                                    AND json_valid(warnings_json)
                                    AND json_type(warnings_json) = 'array'),
    fallback_reason       TEXT,
    failure_code          TEXT,
    failure_message       TEXT,
    created_at            TEXT NOT NULL,
    updated_at            TEXT NOT NULL,
    started_at            TEXT,
    finished_at           TEXT,

    UNIQUE(conversion_id, attempt_number),
    UNIQUE(conversion_id, id),

    CHECK((engine_name IS NULL) = (engine_version IS NULL)),
    CHECK(engine_name IS NULL OR length(engine_name) BETWEEN 1 AND 128),
    CHECK(engine_version IS NULL OR length(engine_version) BETWEEN 1 AND 128),
    CHECK(route IS NULL OR length(route) BETWEEN 1 AND 128),
    CHECK(classification IS NULL OR length(classification) <= 64),
    CHECK(fallback_reason IS NULL OR length(fallback_reason) <= 128),
    CHECK(failure_code IS NULL OR length(failure_code) BETWEEN 1 AND 128),
    CHECK(failure_message IS NULL OR length(failure_message) BETWEEN 1 AND 1024),
    CHECK((failure_code IS NULL) = (failure_message IS NULL)),

    FOREIGN KEY(conversion_id)
        REFERENCES conversions(id)
        ON DELETE CASCADE
) STRICT;

CREATE TABLE artifacts (
    attempt_id            TEXT NOT NULL,
    kind                  TEXT NOT NULL
                                  CHECK(kind IN ('markdown', 'manifest')),
    relative_path         TEXT NOT NULL,
    media_type            TEXT NOT NULL,
    byte_length           INTEGER NOT NULL
                                  CHECK(byte_length > 0),
    sha256                TEXT NOT NULL
                                  CHECK(length(sha256) = 64
                                    AND sha256 NOT GLOB '*[^0-9a-f]*'),
    created_at            TEXT NOT NULL,

    PRIMARY KEY(attempt_id, kind),

    FOREIGN KEY(attempt_id)
        REFERENCES attempts(id)
        ON DELETE CASCADE,

    CHECK(length(relative_path) <= 512
      AND substr(relative_path, 1, 5) = 'jobs/'
      AND instr(relative_path, '..') = 0
      AND instr(relative_path, '//') = 0
      AND instr(
          relative_path,
          '/attempts/' || attempt_id || '/artifacts/'
      ) > 5
      AND relative_path =
        CASE kind
            WHEN 'markdown' THEN
                substr(relative_path, 1, length(relative_path) - 9) || 'result.md'
            WHEN 'manifest' THEN
                substr(relative_path, 1, length(relative_path) - 13) || 'manifest.json'
        END)
) STRICT;

CREATE INDEX idx_conversions_active
    ON conversions(status)
    WHERE status IN ('queued', 'converting_local', 'finalizing');

CREATE INDEX idx_attempts_fifo
    ON attempts(queue_seq)
    WHERE state = 'queued';
