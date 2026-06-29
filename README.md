# nm-api

Local JSON/JSONL NetworkManager adapter for Shelllist and similar frontends.

`nm-api` is not a human Wi-Fi menu. It exposes a frontend-facing protocol over a command transport while Shelllist owns UI, prompts, forms, and presentation.

Stable responses use protocol envelope v1:

```json
{
  "protocol": "nm-api",
  "version": 1,
  "ok": true,
  "data": {}
}
```

Failures use typed errors:

```json
{
  "protocol": "nm-api",
  "version": 1,
  "ok": false,
  "error": { "code": "secret-required", "message": "...", "details": {} },
  "data": {}
}
```

Current Wi-Fi commands:

```bash
nm-api networks [--cached] [--refresh-cache]
nm-api scan [--stream] [--cache] [--strict] [--timeout <seconds>] [--retries <count>] [--ifname <iface>] [--ssid <ssid>...]
nm-api connect <ssid> [--password-stdin] [--bssid <bssid>] [--hidden] [--key-mgmt <hint>] [--wep-key-type key|phrase]
nm-api connect-target <target-json> [--password-stdin] [--wep-key-type key|phrase]
nm-api saved
nm-api profile delete <path>
nm-api profile autoconnect <path> true|false
nm-api profile mac-randomization <path> true|false
nm-api profile share <path>
nm-api profile send-hostname <path> true|false
nm-api status
nm-api disconnect
nm-api connectivity
```

Compatibility during migration:

- `--json` is accepted as a no-op.
- `list` exists as an access-point listing alias but is not the primary Shelllist API.
- `debug diagnose` and `debug contract-fixture` are debug/unstable surfaces.

Secrets must use `--password-stdin`; argv password transport has been removed.

Runtime files and logs live under `$XDG_RUNTIME_DIR/nm-api` by default. Logging environment variables are `NM_API_LOG_FILE`, `NM_API_LOG`, and `NM_API_STDERR_LOG`.

Development:

```bash
nix develop path:.
just check
```

See [PLAN.md](./PLAN.md) for the migration plan.
