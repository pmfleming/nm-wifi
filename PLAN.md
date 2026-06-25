# nm-wifi plan

Goal: provide a stable, UI-agnostic NetworkManager Wi-Fi backend API for Shelllist and similar frontends.

`nm-wifi` is not trying to become an nmcli replacement. The CLI remains only the process boundary that Shelllist invokes. New work should favor stable JSON contracts and exact connect targets over a broad human-facing command surface.

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

Shelllist owns UI behavior: prompts, choice presentation, portal/browser UX, and human interaction.

## Shelllist API parity plan from nmcli review

The nmcli codebase is a reference for correct NetworkManager behavior, not a request to copy nmcli's command UX. Implement the following as JSON/backend capabilities.

1. **Exact AP/device connect targets**
   - Include AP object path, BSSID, device path, and interface name in JSON network records.
   - `connect-target` must preserve and honor these fields.
   - Shelllist should pass the target object back unchanged.

2. **Saved-profile compatibility matching**
   - Use `AvailableConnections` for visible AP activation.
   - Match saved profiles against AP/device compatibility, not only SSID bytes.
   - Account for saved BSSID restrictions and device constraints.

3. **Password supplied for existing profiles**
   - If Shelllist supplies a password, do not blindly activate stale saved secrets first.
   - Prefer password-aware update/add-and-activate behavior.
   - Avoid persistent broken profiles or clean them up on failure.

4. **Structured secret-required / unsupported results**
   - Do not prompt in `nm-wifi`.
   - Return machine-readable states so Shelllist can ask the user or show unsupported auth.

5. **Validation at the backend boundary**
   - Validate SSID byte length (`1..=32`).
   - Validate BSSID syntax.
   - Reject inconsistent exact AP/device target fields.
   - Provide clear JSON/plain errors.

6. **Device-aware scans**
   - Internally support targeted scans by device and SSID bytes.
   - Expose this through stable JSON/cache behavior as Shelllist needs it, not as a broad nmcli-style command set.

7. **Raw APs plus grouped networks**
   - Preserve grouped `networks --json` for Shelllist list UI.
   - Also expose exact AP identity in each entry so Shelllist can connect to a specific AP/BSSID/band.

8. **Enriched AP metadata**
   - Add nmcli-like machine fields where useful for Shelllist:
     - SSID hex.
     - channel/band.
     - mode.
     - bitrate/bandwidth.
     - detailed security labels.

9. **Connection attributes as target metadata**
   - Support target JSON fields for connection name/private scope if Shelllist needs them.
   - Keep these out of the primary human CLI unless needed for testing/API transport.

10. **Activation timeout parity**
    - Use nmcli's 90 second connect timeout as the backend default.
    - Allow target/API timeout override for Shelllist when needed.

## Implementation sequence

Completed foundation:

1. Added target/device identity fields and backend validation.
2. Honor device path/interface constraints in visible AP lookup, hidden scan, saved activation, active-match polling, and nmcli fallback.
3. Replaced visible saved-profile SSID-only matching with AP-compatible `AvailableConnections` matching.
4. Added repeated targeted scan SSIDs and interface-constrained scans for Shelllist hidden-network flows.
5. Raised activation/nmcli fallback timeout defaults toward nmcli's 90 seconds.
6. Added AP JSON enrichment for channel, band, mode, bitrate, bandwidth, SSID hex, and WPA/RSN flag labels.
7. Added password-aware saved-profile secret update before activation.
8. Added grouped exact `access_points` to network entries so Shelllist can select a specific AP/BSSID/band/device while retaining grouped UI rows.
9. Added target JSON connection metadata (`connection_name`/`name`, `private`) and thread it into D-Bus and nmcli fallback activation.

Completed current phase:

1. Upstreamed Shelllist's `--password-stdin` transport so UI prompts do not expose secrets through argv.
2. Added machine-readable JSON failure reasons (`secret-required`, `authorization-required`, `unsupported-auth`, `validation-error`, `timeout`, `activation-failed`, `unknown`) and made JSON connect failures return a failing process status.
3. Replaced string-based connect error classification with an internal typed `ConnectFailureReason` flow. Low-level D-Bus authorization/unsupported names and known activation paths map to typed reasons; unknown external failures remain `unknown`.
4. Updated Shelllist to parse structured JSON stdout even when `nm-wifi connect* --json` exits nonzero.
5. Suggest captive-portal UX only when NetworkManager connectivity reports portal/limited state.
6. Use NetworkManager `AvailableConnections` plus AP/device/BSSID checks for live `networks --json` saved-profile enrichment.
7. Stop mutating existing saved-profile secrets before activation when a caller supplies a password; prefer add-and-activate/new-profile flows that can be cleaned up on failure.
8. Skip stale saved-profile activation in the `nmcli` fallback when a caller supplied a password.

Next:

1. Improve saved-profile compatibility checks beyond `AvailableConnections` for cached/offline records where possible.
2. Add tests around connection metadata serialization and grouped AP output shape.
3. Re-run `cargo fmt`, `cargo clippy`, `cargo test`, and `rust-quality-lens` after each phase.

## Current transport CLI

```bash
nm-wifi list [--cached] [--json] [--refresh-cache]
nm-wifi networks [--cached] [--json] [--refresh-cache]
nm-wifi scan [--stream] [--cache] [--strict] [--timeout <seconds>] [--retries <count>] [--ifname <iface>] [--ssid <ssid>...]
nm-wifi connect <ssid> [--password <secret>|--password-stdin] [--bssid <bssid>] [--hidden] [--wep-key-type key|phrase] [--json]
nm-wifi connect-target <target-json> [--password <secret>|--password-stdin] [--wep-key-type key|phrase] [--json]
nm-wifi saved [--json]
nm-wifi profile delete <path>
nm-wifi profile autoconnect <path> true|false
nm-wifi status [--json]
nm-wifi disconnect [--json]
nm-wifi connectivity [--json]
nm-wifi active
```

## Acceptance checks

- Shelllist can call `nm-wifi networks --cached --json` and receive exact AP/device identity.
- Shelllist can pass one network JSON object back to `connect-target` unchanged.
- `connect-target` validates target identity and honors device/AP constraints.
- Saved profile activation chooses AP-compatible profiles.
- Password-supplied activation does not prefer stale saved secrets.
- `nm-wifi scan --stream --cache` emits JSON Lines snapshots and updates cache files.
- `nm-wifi status --json` reports active Wi-Fi details for frontends.
- `cargo fmt -- --check`, `cargo clippy -- -D warnings`, `cargo test`, and `rust-quality-lens measure all --config rqlens.toml` pass.
