# 'Just' Configuration
# Loads .env file for variables to be used in
# in this just file

set dotenv-load := true

# Ignore recipes that are commented out

set ignore-comments := true

# Set shell for Windows OSs:
# If you have PowerShell Core installed and want to use it,
# use `pwsh.exe` instead of `powershell.exe`

set windows-shell := ["powershell.exe", "-NoLogo", "-Command"]

# Set shell for non-Windows OSs:

set shell := ["bash", "-uc"]

export RUST_BACKTRACE := "1"
export RUST_LOG := "info"
export CI := "1"

build:
    cargo build --all-features
    cargo build -r --all-features

b: build

check:
    cargo check --no-default-features --all-targets --workspace 
    cargo check --all-features --all-targets --workspace

c: check

ci:
    just loop . dev

dev: format lint test lint-deps

d: dev

doc:
    cargo +stable doc --no-deps --all-features --workspace --examples

format-dprint:
    dprint fmt

format-cargo:
    cargo fmt --all

format: format-cargo format-dprint

fmt: format

rev:
    cargo insta review

inverse-deps crate:
    cargo tree -e features -i {{ crate }}

lint: check
    cargo clippy --no-default-features -- -D warnings
    cargo clippy --all-targets --all-features -- -D warnings

lint-deps:
    cargo audit
    cargo deny check

loop dir action:
    watchexec -w {{ dir }} -- "just {{ action }}"

test: check lint
    cargo test --all-targets --all-features --workspace --examples 

test-ignored: check lint
    cargo test --all-targets --all-features --workspace --examples -- --ignored

t: test test-ignored

coverage $RUST_BACKTRACE="0":
    cargo tarpaulin --verbose --all-features --workspace --timeout 120 --out Lcov --output-dir coverage
