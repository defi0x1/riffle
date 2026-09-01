.PHONY: lint test build up down migrate fmt provenance validate-onchain

lint:
	@cargo clippy --all-targets --all-features -- -D warnings
	@cargo fmt --check

test:
	@cargo test --all-targets --all-features

build:
	@cargo build --release

up:
	@docker compose up -d

down:
	@docker compose down

migrate:
	@sqlx migrate run --source migrations

fmt:
	@cargo fmt

provenance:
	@./scripts/provenance-check.sh

# Validates dlmm_tx's instruction builders against the real, deployed DLMM program -- not part
# of `make test` because it needs a local validator or network access. See docs/validation.md.
validate-onchain:
	@./scripts/validate-onchain.sh
