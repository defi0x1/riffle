-- One row per evaluated condition, pass or fail, written for every
-- evaluation including those that emit no signal -- this is what lets
-- /why explain silence. Typed rather than free text so it is
-- diffable across ticks, unlike a competitor's free-text decision log.
CREATE TABLE rationale (
    signal_id            UUID NOT NULL REFERENCES signals (id),
    seq                 INTEGER NOT NULL,
    venue                SMALLINT NOT NULL,
    signal               TEXT NOT NULL,
    observed              TEXT,
    cmp                 TEXT,
    threshold             TEXT,
    passed               BOOLEAN NOT NULL,
    note                 TEXT,
    PRIMARY KEY (signal_id, seq)
);
