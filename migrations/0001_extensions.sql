-- TimescaleDB gives us hypertables, continuous aggregates, compression and
-- retention policies for the time-series layers below. Everything in this
-- migration set assumes it is present.
CREATE EXTENSION IF NOT EXISTS timescaledb;
