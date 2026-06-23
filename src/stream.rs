use std::sync::mpsc::{self, Receiver, Sender};
use std::thread;
use std::time::{Duration, Instant};

use anyhow::{Context, Result};
use zbus::blocking::{Connection, Proxy};

use crate::model::{ScanEvent, ScanStreamOptions, WifiDevice, retry_delay};
use crate::nm::{NM_DEST, Nm, POLL_INTERVAL, WIFI_IFACE};
use crate::output::{StreamOutput, emit_stream_event};

impl Nm {
    pub(crate) fn scan_stream(&self, options: ScanStreamOptions) -> Result<()> {
        let devices = self.wifi_devices()?;
        if devices.is_empty() {
            return emit_empty_device_stream(self, options.cache);
        }

        let rx = spawn_device_watchers(self.connection(), &devices);
        emit_status("preparing scan watchers", options.cache)?;
        emit_empty_snapshot(true, options.cache)?;
        drain_watcher_startup(
            &rx,
            devices.len() * watcher_count_per_device(),
            options.cache,
        )?;

        ScanSession::new(self, rx, devices, options).run()
    }
}

fn emit_status(message: impl Into<String>, cache: bool) -> Result<()> {
    emit_message(MessageKind::Status, message, cache)
}

fn emit_warning(message: impl Into<String>, cache: bool) -> Result<()> {
    emit_message(MessageKind::Warning, message, cache)
}

fn emit_message(kind: MessageKind, message: impl Into<String>, cache: bool) -> Result<()> {
    let message = message.into();
    if cache {
        crate::cache::write_status(kind.cache_state(), &message)?;
    }
    emit_stream_event(&kind.stream_output(message))
}

#[derive(Clone, Copy)]
enum MessageKind {
    Status,
    Warning,
}

impl MessageKind {
    fn cache_state(self) -> &'static str {
        match self {
            MessageKind::Status => "status",
            MessageKind::Warning => "warning",
        }
    }

    fn stream_output(self, message: String) -> StreamOutput<'static> {
        match self {
            MessageKind::Status => StreamOutput::Status { message },
            MessageKind::Warning => StreamOutput::Warning { message },
        }
    }
}

fn emit_empty_snapshot(scanning: bool, cache: bool) -> Result<usize> {
    let networks = Vec::new();
    if cache {
        crate::cache::write_live_scan_snapshot(scanning, &networks)?;
    }
    emit_stream_event(&StreamOutput::Snapshot {
        scanning,
        networks_found: 0,
        networks: &networks,
    })?;
    Ok(0)
}

fn emit_snapshot(nm: &Nm, scanning: bool, cache: bool) -> Result<usize> {
    let networks = nm.list_access_points()?;
    let networks_found = networks.len();
    if cache {
        crate::cache::write_live_scan_snapshot(scanning, &networks)?;
    }
    emit_stream_event(&StreamOutput::Snapshot {
        scanning,
        networks_found,
        networks: &networks,
    })?;
    Ok(networks_found)
}

struct DeviceScanState {
    device: WifiDevice,
    before: i64,
    completed: bool,
    attempts: u32,
    next_retry: Option<Instant>,
}

struct ScanSession<'a> {
    nm: &'a Nm,
    rx: Receiver<ScanEvent>,
    states: Vec<DeviceScanState>,
    options: ScanStreamOptions,
    deadline: Instant,
    last_status: Instant,
    networks_found: usize,
    timed_out: bool,
}

impl<'a> ScanSession<'a> {
    fn new(
        nm: &'a Nm,
        rx: Receiver<ScanEvent>,
        devices: Vec<WifiDevice>,
        options: ScanStreamOptions,
    ) -> Self {
        Self {
            nm,
            rx,
            states: devices
                .into_iter()
                .map(|device| DeviceScanState {
                    before: nm.last_scan(&device),
                    device,
                    completed: false,
                    attempts: 0,
                    next_retry: Some(Instant::now()),
                })
                .collect(),
            options,
            deadline: Instant::now() + options.timeout,
            last_status: Instant::now(),
            networks_found: 0,
            timed_out: false,
        }
    }

    fn run(mut self) -> Result<()> {
        while self.states.iter().any(|state| !state.completed) {
            if self.stop_on_deadline()? {
                break;
            }
            self.retry_due_scan_requests()?;
            if self.recv_and_handle_event()? {
                break;
            }
            self.emit_periodic_status()?;
        }
        self.finish()
    }

    fn stop_on_deadline(&mut self) -> Result<bool> {
        if Instant::now() < self.deadline {
            return Ok(false);
        }
        self.timed_out = true;
        emit_warning(
            format!(
                "scan timed out after {}s; showing latest NetworkManager results",
                self.options.timeout.as_secs()
            ),
            self.options.cache,
        )?;
        Ok(true)
    }

    fn retry_due_scan_requests(&mut self) -> Result<()> {
        let max_attempts = self.options.retries.saturating_add(1);
        let now = Instant::now();
        for index in 0..self.states.len() {
            if retry_is_due(&self.states[index], now, max_attempts) {
                self.try_request_scan(index, now, max_attempts)?;
            }
        }
        Ok(())
    }

    fn try_request_scan(&mut self, index: usize, now: Instant, max_attempts: u32) -> Result<()> {
        self.states[index].attempts += 1;
        match self.nm.request_scan(&self.states[index].device) {
            Ok(()) => self.note_scan_requested(index, max_attempts),
            Err(err) => self.note_scan_request_failed(index, now, max_attempts, err),
        }
    }

    fn note_scan_requested(&mut self, index: usize, max_attempts: u32) -> Result<()> {
        let state = &mut self.states[index];
        state.next_retry = None;
        emit_status(
            format!(
                "requested scan on {} (attempt {}/{max_attempts})",
                state.device.iface, state.attempts
            ),
            self.options.cache,
        )
    }

    fn note_scan_request_failed(
        &mut self,
        index: usize,
        now: Instant,
        max_attempts: u32,
        err: anyhow::Error,
    ) -> Result<()> {
        let state = &mut self.states[index];
        if state.attempts < max_attempts {
            let delay = retry_delay(state.attempts);
            state.next_retry = Some(now + delay);
            return emit_warning(
                format!(
                    "scan request on {} failed: {err:#}; retrying in {}s",
                    state.device.iface,
                    delay.as_secs()
                ),
                self.options.cache,
            );
        }
        state.next_retry = None;
        emit_warning(
            format!(
                "scan request on {} failed after {max_attempts} attempts: {err:#}; continuing with cached results",
                state.device.iface
            ),
            self.options.cache,
        )
    }

    fn recv_and_handle_event(&mut self) -> Result<bool> {
        match self.rx.recv_timeout(self.remaining_wait()) {
            Ok(event) => self.handle_event(event).map(|_| false),
            Err(mpsc::RecvTimeoutError::Timeout) => Ok(false),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                emit_warning("D-Bus watcher channel disconnected", self.options.cache)?;
                Ok(true)
            }
        }
    }

    fn remaining_wait(&self) -> Duration {
        POLL_INTERVAL.min(self.deadline.saturating_duration_since(Instant::now()))
    }

    fn handle_event(&mut self, event: ScanEvent) -> Result<()> {
        match event {
            ScanEvent::WatcherReady => Ok(()),
            ScanEvent::WatcherWarning(message) => emit_warning(message, self.options.cache),
            ScanEvent::AccessPointsChanged => self.refresh_snapshot(),
            ScanEvent::LastScanChanged { device_path, value } => {
                self.mark_completed_device(&device_path, value);
                self.refresh_snapshot()
            }
        }
    }

    fn mark_completed_device(&mut self, device_path: &str, value: i64) {
        for state in &mut self.states {
            if last_scan_matches(state, device_path, value) {
                state.completed = true;
            }
        }
    }

    fn refresh_snapshot(&mut self) -> Result<()> {
        self.networks_found = emit_snapshot(self.nm, true, self.options.cache)?;
        Ok(())
    }

    fn emit_periodic_status(&mut self) -> Result<()> {
        if self.last_status.elapsed() < Duration::from_secs(1) {
            return Ok(());
        }
        let pending = self.states.iter().filter(|state| !state.completed).count();
        emit_status(
            format!(
                "scanning; {} networks found; {pending} devices pending",
                self.networks_found
            ),
            self.options.cache,
        )?;
        self.last_status = Instant::now();
        Ok(())
    }

    fn finish(mut self) -> Result<()> {
        self.networks_found = emit_snapshot(self.nm, false, self.options.cache)?;
        if self.options.cache {
            crate::cache::write_complete(self.timed_out, self.networks_found)?;
        }
        emit_stream_event(&StreamOutput::Complete {
            timed_out: self.timed_out,
            networks_found: self.networks_found,
        })
    }
}

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

fn emit_empty_device_stream(nm: &Nm, cache: bool) -> Result<()> {
    emit_warning(
        "no Wi-Fi devices found; showing cached NetworkManager results",
        cache,
    )?;
    let networks_found = emit_snapshot(nm, false, cache)?;
    if cache {
        crate::cache::write_complete(false, networks_found)?;
    }
    emit_stream_event(&StreamOutput::Complete {
        timed_out: false,
        networks_found,
    })
}

fn spawn_device_watchers(conn: Connection, devices: &[WifiDevice]) -> Receiver<ScanEvent> {
    let (tx, rx) = mpsc::channel();
    for device in devices {
        for kind in watch_kinds() {
            spawn_watcher(watcher_spec(&conn, device, kind, &tx));
        }
    }
    rx
}

fn watch_kinds() -> [WatchKind; 3] {
    [
        WatchKind::AccessPoint("AccessPointAdded"),
        WatchKind::AccessPoint("AccessPointRemoved"),
        WatchKind::LastScan,
    ]
}

fn watcher_count_per_device() -> usize {
    watch_kinds().len()
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
    let _ = spec.tx.send(ScanEvent::WatcherReady);
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
    for _signal in &mut signals {
        let _ = tx.send(ScanEvent::AccessPointsChanged);
    }
    Ok(())
}

fn watch_last_scan(proxy: &Proxy<'_>, spec: &WatcherSpec) -> Result<()> {
    let mut changes = proxy.receive_property_changed::<i64>("LastScan");
    for change in &mut changes {
        let value = change.get().context("read changed LastScan")?;
        let _ = spec.tx.send(ScanEvent::LastScanChanged {
            device_path: spec.device_path.clone(),
            value,
        });
    }
    Ok(())
}

fn drain_watcher_startup(
    rx: &Receiver<ScanEvent>,
    expected_ready: usize,
    cache: bool,
) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(1);
    let mut ready = 0;
    while ready < expected_ready && Instant::now() < deadline {
        ready += drain_one_startup_event(rx, deadline, cache)?;
    }
    warn_if_watchers_missing(ready, expected_ready, cache)
}

fn drain_one_startup_event(
    rx: &Receiver<ScanEvent>,
    deadline: Instant,
    cache: bool,
) -> Result<usize> {
    let remaining = deadline.saturating_duration_since(Instant::now());
    match rx.recv_timeout(POLL_INTERVAL.min(remaining)) {
        Ok(ScanEvent::WatcherReady) => Ok(1),
        Ok(ScanEvent::WatcherWarning(message)) => {
            emit_warning(message, cache)?;
            Ok(0)
        }
        Ok(_) | Err(mpsc::RecvTimeoutError::Timeout | mpsc::RecvTimeoutError::Disconnected) => {
            Ok(0)
        }
    }
}

fn warn_if_watchers_missing(ready: usize, expected_ready: usize, cache: bool) -> Result<()> {
    if ready >= expected_ready {
        return Ok(());
    }
    emit_warning(
        format!("only {ready}/{expected_ready} D-Bus scan watchers became ready before scan start"),
        cache,
    )
}

fn retry_is_due(state: &DeviceScanState, now: Instant, max_attempts: u32) -> bool {
    !state.completed
        && state.attempts < max_attempts
        && state.next_retry.is_some_and(|next_retry| now >= next_retry)
}

fn last_scan_matches(state: &DeviceScanState, device_path: &str, value: i64) -> bool {
    state.device.path.as_str() == device_path && value != state.before && value >= 0
}

impl WatchKind {
    fn label(self) -> &'static str {
        match self {
            WatchKind::AccessPoint(signal_name) => signal_name,
            WatchKind::LastScan => "LastScan",
        }
    }
}
