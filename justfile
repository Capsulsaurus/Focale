# Task runner for Focale. `just check` mirrors CI exactly.

default: check

# Everything CI runs, in the same order.
check: fmt-check lint test

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

lint:
    cargo clippy --workspace --all-targets -- -D warnings

test:
    cargo test --workspace

build:
    cargo build --workspace

run *ARGS:
    cargo run -p focale-app -- {{ARGS}}
