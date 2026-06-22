set dotenv-load := false

check:
    cargo fmt -- --check
    cargo clippy -- -D warnings
    cargo test

fmt:
    cargo fmt

run *args:
    cargo run -- {{args}}

list:
    cargo run -- list

scan:
    cargo run -- scan --timeout 20
