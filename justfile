set shell := ["bash", "-uc"]

default:
    @just --list

fmt:
    cargo fmt --all

fetch:
    cargo fetch --locked

fmt-check:
    cargo fmt --all -- --check

lint:
    cargo clippy --locked --all-targets --all-features -- -D warnings

test:
    cargo test --locked --all-targets --all-features

release:
    cargo build --release --locked

help:
    cargo run --locked -- --help

run-hosts config="fixtures/config/valid.yaml":
    cargo run --locked -- --config {{config}} hosts

run-list config="fixtures/config/pi.yaml":
    cargo run --locked -- --config {{config}} list

validate-fixtures:
    cargo run --locked -- --config fixtures/config/pi.yaml hosts
    ! cargo run --locked -- --config fixtures/config/duplicate-host.yaml hosts
    ! cargo run --locked -- --config fixtures/config/duplicate-session.yaml hosts
    ! cargo run --locked -- --config fixtures/config/missing-host-ref.yaml hosts
    ! cargo run --locked -- --config fixtures/config/invalid-duration.yaml hosts

ci: fetch fmt-check lint test release help validate-fixtures

# bump version (patch|minor|major|X.Y.Z), commit, tag; push manually after
release bump:
    scripts/release.sh {{bump}}
