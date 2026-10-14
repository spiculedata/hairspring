.PHONY: build test lint fmt ui release audit docker clean

build:
	cargo build --workspace

test:
	cargo test --workspace --no-fail-fast

lint:
	cargo clippy --workspace --all-targets -- -D warnings

fmt:
	cargo fmt --all

# The UI is embedded into the server binary at compile time: build it before a release build.
ui:
	cd web && npm ci && npm run build

release: ui
	cargo build --release -p hairspring-cli -p hairspring-dapr

audit:
	cargo audit
	cd web && npm audit --omit=dev

docker:
	docker compose build

clean:
	cargo clean
	rm -rf web/dist/* web/node_modules
	touch web/dist/.gitkeep
