.PHONY: build test lint fmt ui check-ui release audit docker clean test-scripts

build:
	cargo build --workspace

test:
	cargo test --workspace --no-fail-fast

lint:
	cargo clippy --workspace --all-targets -- -D warnings

# The install scripts: lint and test them with fake commands (add --mutations to prove the tests can fail).
test-scripts:
	shellcheck -S warning scripts/*.sh
	bash scripts/test-scripts.sh

fmt:
	cargo fmt --all

# The UI is embedded into the server binary at compile time: build it before a release build.
ui:
	cd web && npm ci && npm run build

# After a release build: the binary must serve the UI built in web/dist, not an older one.
check-ui:
	bash scripts/check-embedded-ui.sh target/release/hairspring web/dist

release: ui
	cargo build --release -p hairspring-cli -p hairspring-dapr
	$(MAKE) check-ui

audit:
	cargo audit
	cd web && npm audit --omit=dev

docker:
	docker compose build

clean:
	cargo clean
	rm -rf web/dist/* web/node_modules
	touch web/dist/.gitkeep
