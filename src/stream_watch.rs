use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;

use anyhow::{Context, Result};
use zbus::blocking::{Connection, Proxy};

use crate::model::{ScanEvent, WifiDevice};
use crate::nm::{NM_DEST, WIFI_IFACE};

#[derive(Clone, Copy)]
enum WatchKind {
    AccessPoint(&'static str),
    LastScan,
}

struct WatcherSpec {
    conn: Connection,
    path: zvariant::OwnedObjectPath,
    iface: String,
    device_path: String,
    kind: WatchKind,
    tx: Sender<ScanEvent>,
}

pub(crate) fn spawn_device_watchers(
    conn: Connection,
    devices: &[WifiDevice],
) -> Receiver<ScanEvent> {
    let (tx, rx) = mpsc::channel();
    for device in devices {
        for kind in watch_kinds() {
            spawn_watcher(watcher_spec(&conn, device, kind, &tx));
        }
    }
    rx
}

pub(crate) fn watcher_count_per_device() -> usize {
    watch_kinds().len()
}

fn watch_kinds() -> [WatchKind; 3] {
    [
        WatchKind::AccessPoint("AccessPointAdded"),
        WatchKind::AccessPoint("AccessPointRemoved"),
        WatchKind::LastScan,
    ]
}

fn watcher_spec(
    conn: &Connection,
    device: &WifiDevice,
    kind: WatchKind,
    tx: &Sender<ScanEvent>,
) -> WatcherSpec {
    WatcherSpec {
        conn: conn.clone(),
        path: device.path.clone(),
        iface: device.iface.clone(),
        device_path: device.path.to_string(),
        kind,
        tx: tx.clone(),
    }
}

fn spawn_watcher(spec: WatcherSpec) {
    thread::spawn(move || {
        if let Err(err) = run_watcher(&spec) {
            let _ = spec.tx.send(ScanEvent::WatcherWarning(format!(
                "{} watcher for {} failed: {err:#}",
                spec.kind.label(),
                spec.iface
            )));
        }
    });
}

fn run_watcher(spec: &WatcherSpec) -> Result<()> {
    let proxy = Proxy::new(&spec.conn, NM_DEST, spec.path.as_str(), WIFI_IFACE)
        .context("create Wi-Fi watcher proxy")?;
    match spec.kind {
        WatchKind::AccessPoint(signal_name) => watch_access_points(&proxy, signal_name, &spec.tx),
        WatchKind::LastScan => watch_last_scan(&proxy, spec),
    }
}

fn watch_access_points(
    proxy: &Proxy<'_>,
    signal_name: &'static str,
    tx: &Sender<ScanEvent>,
) -> Result<()> {
    let mut signals = proxy
        .receive_signal(signal_name)
        .with_context(|| format!("receive {signal_name}"))?;
    let _ = tx.send(ScanEvent::WatcherReady);
    for _signal in &mut signals {
        let _ = tx.send(ScanEvent::AccessPointsChanged);
    }
    Ok(())
}

fn watch_last_scan(proxy: &Proxy<'_>, spec: &WatcherSpec) -> Result<()> {
    let mut changes = proxy.receive_property_changed::<i64>("LastScan");
    let _ = spec.tx.send(ScanEvent::WatcherReady);
    for change in &mut changes {
        let value = change.get().context("read changed LastScan")?;
        let _ = spec.tx.send(ScanEvent::LastScanChanged {
            device_path: spec.device_path.clone(),
            value,
        });
    }
    Ok(())
}

impl WatchKind {
    fn label(self) -> &'static str {
        match self {
            WatchKind::AccessPoint(signal_name) => signal_name,
            WatchKind::LastScan => "LastScan",
        }
    }
}
