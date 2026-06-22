# nm-wifi-rofi-rust

Rust/NetworkManager D-Bus replacement for the rofi Wi-Fi chooser.

Current status: first D-Bus helper implementation.

Implemented commands:

```bash
nm-wifi-rofi list
nm-wifi-rofi scan --timeout 20
nm-wifi-rofi active
```

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
