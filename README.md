# nm-wifi-rofi-rust

Rust/NetworkManager D-Bus replacement for the rofi Wi-Fi chooser.

Current status: first D-Bus helper implementation with experimental live scan streaming.

Implemented commands:

```bash
nm-wifi-rofi list
nm-wifi-rofi scan --timeout 20
nm-wifi-rofi scan --stream --timeout 20 --retries 2
nm-wifi-rofi scan --strict --timeout 20
nm-wifi-rofi active
```

`scan --stream` emits JSON Lines progress events and repeated snapshots as NetworkManager adds/removes access points. Plain `scan` keeps TSV output and falls back to cached NetworkManager results with a stderr warning unless `--strict` is used.

Development:

```bash
nix develop path:.
just check
```

Or without `just`:

```bash
cargo fmt -- --check
cargo clippy -- -D warnings
cargo test
```

Or without entering the shell:

```bash
nix develop path:. -c just check
```

If you use direnv:

```bash
direnv allow
```

See [PLAN.md](./PLAN.md).
