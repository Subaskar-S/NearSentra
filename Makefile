.PHONY: install test sandbox-deploy services simulate demo-agent-payment lint

VAULT_DIR := contracts/vault

# The Sentinel needs Python 3.11+. macOS ships 3.9, so pass e.g. PYTHON=/opt/homebrew/bin/python3.
PYTHON ?= python3

# Install every component's dependencies (Rust crates are fetched on the first build or test).
install:
	cd services/gateway && npm ci
	cd simulation && npm ci
	cd frontend && npm ci
	test -x services/sentinel/.venv/bin/python || $(PYTHON) -m venv services/sentinel/.venv
	services/sentinel/.venv/bin/python -m ensurepip --upgrade >/dev/null
	services/sentinel/.venv/bin/python -m pip install --quiet --disable-pip-version-check --require-hashes -r services/sentinel/requirements.lock
	@echo "Dependencies installed."

# Run all contract unit tests and sandbox integration tests.
# Fails (non-zero) if any test fails or any expected integration test file is missing.
test:
	@for f in init access_control registration authorization epoch suspension callback agent_to_agent; do \
		test -f $(VAULT_DIR)/tests/test_$$f.rs || { echo "missing sandbox test: test_$$f.rs"; exit 1; }; \
	done
	cd $(VAULT_DIR) && cargo test

# Deploy the vault to a fresh local NEAR sandbox and print its account ID.
# Stays running (Ctrl-C to stop) and writes connection details to .localnet/localnet.env.
# Use `make sandbox-deploy ONCE=1` to deploy, print, and exit.
sandbox-deploy:
	cd $(VAULT_DIR) && cargo run --quiet --example localnet $(if $(ONCE),-- --once,)

# Start the gateway, sentinel and dashboard against the running local network
# (run `make sandbox-deploy` in another terminal first). Ctrl-C stops them.
services:
	./scripts/start-services.sh

# Run the six simulations (five attack scenarios and the agent-to-agent payment) against a throwaway local sandbox.
simulate:
	./scripts/run-simulation.sh

# Agent-to-agent payment through the running Gateway, visible on the dashboard
# (needs `make sandbox-deploy` and `make services` running in other terminals).
demo-agent-payment:
	./scripts/run-agent-payment-demo.sh

# Lint every component.
lint:
	cd $(VAULT_DIR) && cargo clippy --all-targets -- -D warnings
	@if [ -f services/gateway/package.json ]; then cd services/gateway && npx tsc --noEmit; fi
	@if [ -f simulation/package.json ]; then cd simulation && npx tsc --noEmit; fi
	@if [ -f frontend/package.json ]; then cd frontend && npx tsc --noEmit; fi
	@if [ -d services/sentinel ]; then cd services/sentinel && .venv/bin/python -m ruff check .; fi
