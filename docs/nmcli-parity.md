# nmcli parity matrix

`nm-wifi diagnose [--json]` is the local parity probe for the Shelllist-facing
subset of `nmcli` behavior. It compares `nm-wifi`'s D-Bus/cache view with live
`nmcli` output and reports pass/warn/fail/unknown checks.

## Current high-impact matrix

| Area | nmcli reference | nm-wifi surface | Why it matters |
| --- | --- | --- | --- |
| Active SSID | `nmcli -t -f IN-USE,SSID ... dev wifi list --rescan no` | `status --json.access_point.ssid` | Shelllist must highlight the connected network. |
| Active BSSID | same | `status --json.access_point.bssid` | Exact AP selection among same-SSID APs. |
| Active frequency | same | `status --json.access_point.frequency` | Detail pane should show the actual connected band/AP. |
| Signal | same | `status --json.access_point.strength` | UI list/detail signal should agree with NetworkManager. |
| IPv4 address | `nmcli -t device show <iface>` | `status --json.ip4.address` | Connection details card. |
| Gateway | same | `status --json.ip4.gateway` | Connection details card. |
| DNS | same | `status --json.ip4.dns` | Connection details card. |
| Active enriched network | n/a, derived | `networks --json` active grouped entry | Shelllist selection/detail consistency. |
| Remembered details | n/a, nm-wifi cache | `networks --json.last_connection` | Details for previously connected networks. |

## Usage

```bash
nm-wifi diagnose
nm-wifi diagnose --json | jq '.summary, .checks'
```

A clean Shelllist parity run should have no `fail` checks. `warn` usually means
one side is missing a value or signal changed between scans; inspect the check's
`detail` field.

## Closed gaps from the first matrix pass

- Active SSID groups now prefer the active AP before strongest AP fallback.
- `status --json` now reads IPv4 gateway from D-Bus `RouteData` and DNS from
  D-Bus `NameserverData`/legacy `Nameservers`; `nmcli device show <iface>` is
  only a last-resort fill-in when D-Bus IP data is incomplete.
- Connect caching waits briefly for DHCP/IP details before remembering the
  connection.
- Enriched network JSON carries `last_connection` so Shelllist can show cached
  details for previously connected networks.
