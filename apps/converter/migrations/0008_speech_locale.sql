-- Record the per-run transcription language.
--
-- Nullable, so an ALTER is enough. It lives on the conversion for the reason
-- the speaker count does: the runner claims work long after the request is
-- gone, and the value must reach the worker unchanged across a restart. NULL
-- means the Mac's own language, which is every row written before this.

ALTER TABLE conversions ADD COLUMN speech_locale TEXT
    CHECK(speech_locale IS NULL OR length(speech_locale) BETWEEN 2 AND 35);
