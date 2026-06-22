# nm-wifi-rofi-rust plan

Goal: replace the shell/nmcli Wi-Fi listing/rescan path with a Rust D-Bus based helper, then optionally move the full rofi Wi-Fi menu into Rust.

## 1. Scope and constraints

- Use Rust, not Python.
- Use NetworkManager D-Bus for scan/list/status.
- Keep the current rofi UX initially.
- Keep `nmcli` for connection activation/password handling at first to avoid D-Bus secret-agent complexity.
- Integrate with the existing NixOS config only after the helper is stable.

## 2. Initial project setup

- Create a Rust binary crate named `nm-wifi-rofi`.
- Add dependencies:
  - `zbus` for D-Bus
  - `zvariant` if needed for typed D-Bus values
  - `anyhow` for errors
  - `clap` for subcommands
- Add subcommands:
  - `list`
  - `scan`
  - `active`
  - later: `connect`, `rofi`

## 3. D-Bus discovery

Implement NetworkManager object discovery:

1. Connect to the system bus.
2. Query `/org/freedesktop/NetworkManager`.
3. Read `Devices` or call `GetDevices`.
4. For each device, read device type.
5. Keep devices with type `NM_DEVICE_TYPE_WIFI`.

Output useful debug info first:

```text
wifi-device /org/freedesktop/NetworkManager/Devices/3 wlan0
```

## 4. Access point listing

For each Wi-Fi device:

1. Call `GetAccessPoints` or read `AccessPoints`.
2. For each AP object, read:
   - `Ssid`
   - `Strength`
   - `Flags`
   - `WpaFlags`
   - `RsnFlags`
   - `Frequency`
   - `HwAddress`
   - `LastSeen`
3. Decode SSID byte arrays safely.
4. Convert security flags into simple labels/icons.
5. Deduplicate by SSID, keeping strongest signal.
6. Sort by signal descending.

Initial output should be stable TSV:

```text
SSID<TAB>active<TAB>security<TAB>signal<TAB>frequency<TAB>bssid
```

## 5. Active network detection

Use D-Bus instead of parsing `nmcli`:

1. Read `ActiveAccessPoint` from each Wi-Fi device.
2. Compare it with AP object paths.
3. Mark the matching SSID as active.

## 6. Scan implementation

Use proper NetworkManager scan completion:

1. Read current `LastScan` from the Wi-Fi device.
2. Call `RequestScan({})`.
3. Listen for `org.freedesktop.DBus.Properties.PropertiesChanged`.
4. Wait until `LastScan` changes.
5. Add a timeout fallback, e.g. 12 seconds.
6. Print the refreshed list.

Important: NetworkManager still does not provide percentage progress. We can only report start/finish or elapsed time.

## 7. Parallel integration strategy

Do not replace the existing chooser initially.

- Keep the current shell chooser on `SUPER+N` / `rofi-wifi-menu`.
- Add the Rust/D-Bus chooser as a parallel command on `SUPER+M`.
- Use this parallel path while developing and testing.
- Only consider replacing `SUPER+N` after the Rust chooser is clearly better.

Target commands:

```text
SUPER+N → rofi-wifi-menu          # current stable chooser
SUPER+M → nm-wifi-rofi rofi       # new Rust/D-Bus chooser
```

## 8. Integrate with current shell menu, optional fallback path

If we want an intermediate hybrid before full rofi mode, keep the shell menu but replace only data sources:

- Replace `wifi_entries()` in `config/scripts/rofi-wifi-menu.sh` with:

```sh
nm-wifi-rofi list
```

- Replace rescan implementation with:

```sh
nm-wifi-rofi scan
```

Keep existing shell code for:

- saved profiles
- password prompt
- connection attempts
- captive portal handling

This is optional; the preferred path is a parallel full chooser on `SUPER+M`.

## 9. Package with Nix

Add a package under `/etc/nixos/packages/nm-wifi-rofi` or similar.

Options:

- `rustPlatform.buildRustPackage`
- vendor dependencies with `cargoHash`

Then add the package to `rofiWifiMenu.runtimeInputs`.

## 10. Full rofi-mode rewrite

Once the D-Bus helper is stable, move the entire rofi Wi-Fi script into Rust.

Rust would emit rofi script-mode rows directly and read:

- `ROFI_RETV`
- `ROFI_INFO`
- `ROFI_DATA`

Actions:

- default: emit menu
- `rescan`: request scan, wait for `LastScan`, emit menu
- `portal`: launch captive portal browser
- `connect:<ssid>`: initially shell out to `nmcli`

## 11. Optional full D-Bus connection activation

Only do this later.

Needed pieces:

- list saved `Settings.Connection` profiles over D-Bus
- match profiles by SSID
- call `ActivateConnection`
- create new Wi-Fi connection for unknown SSIDs
- handle secrets/passwords correctly

This is more complex than listing/scanning and should not block the first version.

## 12. Test checklist

- `nm-wifi-rofi list` works with Wi-Fi enabled.
- `nm-wifi-rofi list` handles Wi-Fi disabled gracefully.
- `nm-wifi-rofi scan` waits for `LastScan` change.
- Hidden/invalid UTF-8 SSIDs do not crash the program.
- Duplicate SSIDs collapse to the strongest AP.
- Active SSID is marked correctly.
- Existing rofi connect flow still works.
- Nix build succeeds.

## 13. Success criteria

- No `nmcli device wifi list` parsing remains in the rofi menu.
- Rescan completion is based on `LastScan`, not arbitrary sleep.
- Existing user-visible Wi-Fi behavior remains unchanged or improves.
- Code is small, typed, and testable.
