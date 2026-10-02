-- no-transaction
-- Record the per-run speaker count and admit the `audio` attempt
-- classification the Audio engine writes.
--
-- The speaker count is a new nullable column, so `conversions` takes an ALTER
-- rather than the rebuild migrations 0002 to 0005 needed: only a CHECK on an
-- existing column forces a copy, and `source_media_type` is unchanged here.
-- That leaves migration 0005 the newest declaration of the media-type CHECK,
-- which is the one `advertised_media_types_match_the_migration_check` parses.
--
-- `attempts.classification` is a CHECK, so that table is rebuilt and copied
-- exactly as 0003 did. Foreign keys are disabled for the rebuild and
-- re-enabled after; the copy preserves rowids, so attempts.queue_seq keeps its
-- AUTOINCREMENT high-water mark. Only `idx_attempts_fifo` is recreated: an
-- index dies with its table and recreating a surviving one is an error.
--
-- The speaker count lives on the conversion for the reason the OCR settings
-- do: the runner claims work from this table long after the request is gone,
-- and the value must reach the worker unchanged across a restart. NULL means
-- the diarizer guesses, which is every row written before this migration.

PRAGMA foreign_keys = OFF;

-- The rebuild is one transaction. sqlx runs this file in autocommit, so
-- without it a kill between DROP and RENAME leaves an unbootable schema.
BEGIN;

ALTER TABLE conversions ADD COLUMN speaker_count INTEGER
    CHECK(speaker_count IS NULL OR speaker_count BETWEEN 1 AND 20);

CREATE TABLE attempts_new (
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
                                            'mixed',
                                            'structured_document',
                                            'audio'
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

INSERT INTO attempts_new SELECT * FROM attempts;
DROP TABLE attempts;
ALTER TABLE attempts_new RENAME TO attempts;

CREATE INDEX idx_attempts_fifo
    ON attempts(queue_seq)
    WHERE state = 'queued';

COMMIT;

PRAGMA foreign_keys = ON;
