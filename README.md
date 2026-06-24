# nm-wifi

UI-agnostic NetworkManager D-Bus Wi-Fi helper.

`nm-wifi` provides scanning, cached JSON/JSONL output, connection activation, saved-profile management, and active-network reporting for frontends such as Shelllist. Rofi-specific menu rendering has been removed; Shelllist is now the intended desktop Wi-Fi UI.

Implemented commands:

```bash
nm-wifi list
nm-wifi list --cached --json
nm-wifi networks --cached --json
nm-wifi scan --timeout 20
nm-wifi scan --stream --cache --timeout 20 --retries 2
nm-wifi scan --strict --timeout 20
nm-wifi connect <ssid>
nm-wifi connect <ssid> --password <password>
nm-wifi connect <ssid> --bssid <bssid>
nm-wifi connect <ssid> --hidden --password <password>
nm-wifi connect <ssid> --password <wep-key> --wep-key-type key
nm-wifi connect-target '{"ssid":"Cafe","ssid_bytes":[67,97,102,101],"path":"/org/..."}' --json
nm-wifi saved --json
nm-wifi profile delete <path>
nm-wifi profile autoconnect <path> true|false
nm-wifi connectivity --json
nm-wifi active
```

`networks --json` enriches visible access points with saved-profile matches and backend connection capabilities for UI frontends. `connect-target` accepts one of those JSON objects directly, preserving exact SSID bytes and AP object paths. Add `--json` to `connect` or `connect-target` to emit a structured connection result with connectivity state and portal recommendation.

`scan --stream` emits JSON Lines progress events and repeated enriched snapshots as NetworkManager adds/removes access points. Add `--cache` to write `latest.json`, `scan-session.json`, and `status.json` under `$XDG_RUNTIME_DIR/nm-wifi`.

Prefer `--json` for stable machine-readable output. JSON includes display SSIDs plus raw `ssid_bytes`; plain TSV output is intended for humans and escapes tabs, newlines, backslashes, NUL, and control characters.

Connection activation uses NetworkManager D-Bus for saved Wi-Fi profiles, passwordless visible networks, hidden SSIDs, WEP networks, and WPA/WPA2/WPA3-Personal networks, with `nmcli` retained as a fallback for unsupported edge cases such as enterprise flows. `connectivity --json` exposes NetworkManager connectivity state for frontends.

Logging:

- Detailed logs are written by default to `$XDG_RUNTIME_DIR/nm-wifi/nm-wifi.log`.
- Use `--log-file <path>` or `NM_WIFI_LOG_FILE=<path>` to choose another file.
- Use `-v`/`-vv` for more stderr logging; use `NM_WIFI_LOG` and `NM_WIFI_STDERR_LOG` for tracing filters.
- Passwords are redacted from logged `nmcli` arguments.

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
