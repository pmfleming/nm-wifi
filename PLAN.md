# nm-wifi plan

Goal: provide a UI-agnostic NetworkManager Wi-Fi backend for desktop frontends.

## Scope

`nm-wifi` owns NetworkManager-specific behavior:

- Wi-Fi device discovery.
- access-point list and scan requests.
- cached snapshots and scan status under `$XDG_RUNTIME_DIR/nm-wifi`.
- JSON and JSON Lines machine-readable output.
- saved-profile listing, delete, and autoconnect changes.
- connection activation over D-Bus with `nmcli` fallback for unsupported edge cases.
- SSID byte preservation for non-UTF-8/hidden networks.
- logging and structured backend status.

Frontends own UI behavior. Shelllist is the intended Quickshell frontend and should be bound to the desktop Wi-Fi shortcut, replacing the previous rofi Wi-Fi menu on `SUPER+N`.

## Current CLI

```bash
nm-wifi list [--cached] [--json] [--refresh-cache]
nm-wifi networks [--cached] [--json] [--refresh-cache]
nm-wifi scan [--stream] [--cache] [--strict] [--timeout <seconds>] [--retries <count>]
nm-wifi connect <ssid> [--password <secret>] [--bssid <bssid>] [--hidden] [--wep-key-type key|phrase] [--json]
nm-wifi connect-target <target-json> [--password <secret>] [--wep-key-type key|phrase] [--json]
nm-wifi saved [--json]
nm-wifi profile delete <path>
nm-wifi profile autoconnect <path> true|false
nm-wifi status [--json]
nm-wifi disconnect [--json]
nm-wifi connectivity [--json]
nm-wifi active
```

## Removed scope

Rofi script-mode rendering and callback/action encoding have been removed from this backend. The old rofi-specific modules were frontend code and belong outside `nm-wifi`.

## Next backend/API improvements

1. Keep browser/captive-portal launching in Shelllist unless another frontend needs shared browser UX.

## Acceptance checks

- `nm-wifi list --json` works with Wi-Fi enabled.
- `nm-wifi list --json` handles Wi-Fi disabled gracefully.
- `nm-wifi scan --stream --cache` emits JSON Lines snapshots and updates cache files.
- `nm-wifi networks --cached --json` includes saved-profile matches and capabilities.
- `nm-wifi saved --json` lists saved Wi-Fi profiles.
- `nm-wifi connect-target <target-json> --json` accepts enriched network JSON from frontends and returns a structured connection result.
- `nm-wifi status --json` reports active Wi-Fi details for frontends.
- `nm-wifi disconnect --json` deactivates the active Wi-Fi connection.
- `nm-wifi connectivity --json` reports NetworkManager connectivity state.
- `nm-wifi profile delete` and `profile autoconnect` manage profiles by D-Bus object path.
- `nm-wifi connect` activates saved, open, hidden, WEP, and PSK networks where supported.
- `cargo fmt -- --check`, `cargo clippy -- -D warnings`, and `cargo test` pass.
