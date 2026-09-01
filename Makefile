.PHONY: lint test build up down migrate fmt provenance

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
