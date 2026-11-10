# DeckPilot development commands.
#
# Recipes only call cargo and pnpm, so they work wherever GNU Make is
# available (Linux, macOS, and Windows through Git Bash or MSYS2). Every
# target is a thin wrapper; `cargo run -p deckpilot-dev -- <command>` works
# without Make as well.

DEV := cargo run --quiet -p deckpilot-dev --
DEVICES ?= mk2,xl,plus

.PHONY: help setup up down restart status logs agent ui seed test lint check build package clean

help:
	@echo "DeckPilot development commands"
	@echo ""
	@echo "  make setup     Install dependencies and check the toolchain"
	@echo "  make up        Start agent + app with live reload (simulated devices: DEVICES=$(DEVICES))"
	@echo "  make down      Stop the development session (data is kept)"
	@echo "  make restart   Restart the session (rebuilds the agent)"
	@echo "  make status    Show session processes and agent state"
	@echo "  make logs      Follow agent and app logs"
	@echo "  make agent     Run only the agent in the foreground"
	@echo "  make ui        Run only the app with live reload in the foreground"
	@echo "  make seed      Add demonstration data to an empty dev database"
	@echo "  make test      Run all tests"
	@echo "  make lint      Formatting, Clippy, ESLint and type checks"
	@echo "  make check     lint + test"
	@echo "  make build     Debug build of all components"
	@echo "  make package   Installers for this platform, including the agent"
	@echo "  make clean     Remove build output, logs and session files"
	@echo ""
	@echo "Development data lives in .dev/ and never touches your real configuration."

setup:
	pnpm install --frozen-lockfile
	$(DEV) doctor
	cargo build -p deckpilot-agent

up:
	$(DEV) up --devices $(DEVICES)

down:
	$(DEV) down

restart:
	$(DEV) restart

status:
	$(DEV) status

logs:
	$(DEV) logs -f

agent:
	$(DEV) agent --devices $(DEVICES)

ui:
	$(DEV) ui

seed:
	$(DEV) seed

test:
	cargo test --workspace --locked
	pnpm test

lint:
	cargo fmt --all --check
	cargo clippy --workspace --all-targets --locked -- -D warnings
	pnpm format:check
	pnpm lint
	pnpm typecheck

check: lint test
	$(DEV) check-versions

build:
	pnpm --filter @deckpilot/desktop build
	cargo build --workspace --locked

package:
	$(DEV) package

clean:
	$(DEV) clean
	cargo clean
