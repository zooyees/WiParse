//! Private station execution primitives; deliberately not an API capability.
//!
//! Hardware ownership, approved device rules and a capture adapter with
//! verified loss reporting are not yet integrated. These primitives alone do
//! not make live station operation safe or available.

use crate::wireless_gate::{DispatchError, Gate, GateError, Identity};
use rusqlite::{params, Connection, TransactionBehavior};
use std::{fs::File, io::{Read, Write}, path::Path, sync::{Arc, Mutex, atomic::{AtomicBool, Ordering}}, time::Instant};

/// Sole owner of a capture's byte stream and gate state. A real adapter must
/// provide a trustworthy quality signal; the existing UART API explicitly
/// lacks line-error and drop counters and therefore cannot assert it.
pub(crate) struct Capture {
    identity: Identity,
    gate: Gate,
    pending: Vec<u8>,
    next_line: u64,
    failed: bool,
}

impl Capture {
    pub(crate) fn new(identity: Identity, gate: Gate, first_line: u64) -> Self {
        Self { identity, gate, pending: Vec::new(), next_line: first_line, failed: false }
    }

    /// Feed bytes in source order, only from the station-owned reader. Failure
    /// to verify continuity, partial-line overflow, or I/O failure closes the
    /// attempt; a subsequent chunk cannot reopen it.
    pub(crate) fn ingest(&mut self, bytes: &[u8], continuity_verified: bool,
        evidence: &mut impl Write, now: Instant) -> Result<(), String> {
        if self.failed { return Err("STATION.CAPTURE_CLOSED".into()); }
        if !continuity_verified { return self.fail("STATION.CAPTURE_QUALITY_UNVERIFIED"); }
        // Persist capture bytes before deriving readiness; if evidence storage
        // fails no later line in this attempt may authorize a dispatch.
        if evidence.write_all(bytes).is_err() { return self.fail("STATION.EVIDENCE_WRITE_FAILED"); }
        for &byte in bytes {
            if byte == b'\n' {
                let text = match std::str::from_utf8(&self.pending) {
                    Ok(text) => text.trim_end_matches('\r'),
                    Err(_) => return self.fail("STATION.CAPTURE_ENCODING"),
                };
                if let Err(error) = self.gate.observe_line(&self.identity, self.next_line, text, true, now) {
                    self.failed = true;
                    return Err(format!("STATION.GATE_{error:?}"));
                }
                self.next_line = match self.next_line.checked_add(1) {
                    Some(next) => next,
                    None => return self.fail("STATION.CAPTURE_SEQUENCE_OVERFLOW"),
                };
                self.pending.clear();
            } else {
                self.pending.push(byte);
                if self.pending.len() > 64 * 1024 { return self.fail("STATION.CAPTURE_LINE_TOO_LONG"); }
            }
        }
        Ok(())
    }

    /// Poll an already-owned UART connection once. `quality_verified` must be
    /// obtained from the trusted adapter, never a client argument: the current
    /// generic serial backend lacks error/drop counters and must pass false.
    /// The same owner must serialize polling, invalidation and dispatch.
    pub(crate) fn poll_uart(&mut self, port: &mut impl Read, evidence: &mut File,
        mut quality_verified: impl FnMut() -> bool, mut lease_valid: impl FnMut() -> bool, cancelled: &AtomicBool,
        deadline: Instant) -> Result<usize, String> {
        if self.failed { return Err("STATION.CAPTURE_CLOSED".into()); }
        if cancelled.load(Ordering::SeqCst) { return self.fail("STATION.CANCELLED"); }
        if !lease_valid() { return self.fail("STATION.LEASE_LOST"); }
        if !quality_verified() { return self.fail("STATION.CAPTURE_QUALITY_UNVERIFIED"); }
        if Instant::now() >= deadline { return self.fail("STATION.CAPTURE_TIMEOUT"); }
        let mut bytes = [0u8; 8192];
        let n = match port.read(&mut bytes) {
            Ok(0) => return self.fail("STATION.UART_DISCONNECTED"),
            Ok(n) => n,
            Err(error) if matches!(error.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => return Ok(0),
            Err(_) => return self.fail("STATION.UART_READ_FAILED"),
        };
        if cancelled.load(Ordering::SeqCst) || !lease_valid() || !quality_verified() || Instant::now() >= deadline {
            return self.fail("STATION.CAPTURE_STOPPED_BEFORE_EVIDENCE");
        }
        self.ingest(&bytes[..n], true, evidence, Instant::now())?;
        // Dispatch follows only after the capture owner returns; persist bytes
        // first so a matched readiness line has an evidence file on disk.
        if evidence.sync_data().is_err() { return self.fail("STATION.EVIDENCE_SYNC_FAILED"); }
        Ok(n)
    }

    pub(crate) fn stop(&mut self) {
        self.failed = true;
        self.gate.invalidate(GateError::CaptureDiscontinuous);
    }
    fn fail<T>(&mut self, reason: &str) -> Result<T, String> {
        self.stop();
        Err(reason.into())
    }
}

/// Durable at-most-once intent ledger. It stores identity and command IDs,
/// never raw TX log lines, device commands or credentials.
pub(crate) struct Ledger { db: Connection }

/// Internal ownership boundary for a single station attempt. Capture updates
/// and dispatch share one lock; stop marks cancellation before waiting on it.
/// A real adapter must hold the physical device session for this lifetime.
pub(crate) struct StationAttempt {
    capture: Mutex<Capture>,
    ledger: Mutex<Ledger>,
    cancelled: AtomicBool,
    device: String,
}

/// Connects a station attempt to the existing service lease manager; this
/// fences only cooperating service jobs, not GUI sessions in another process.
pub(crate) struct ServiceLease {
    state: Arc<Mutex<crate::jobs::State>>,
    resource: String,
    token: String,
    epoch: String,
}

impl ServiceLease {
    pub(crate) fn new(state: Arc<Mutex<crate::jobs::State>>, resource: String,
        token: String, epoch: String) -> Result<Self, String> {
        let lease = Self { state, resource, token, epoch };
        if !lease.valid() { return Err("STATION.LEASE_INVALID".into()); }
        Ok(lease)
    }
    pub(crate) fn valid(&self) -> bool {
        crate::jobs::station_lease_valid(&self.state, &self.resource, &self.token, &self.epoch)
    }
    pub(crate) fn resource(&self) -> &str { &self.resource }
    pub(crate) fn epoch(&self) -> &str { &self.epoch }
}

impl StationAttempt {
    pub(crate) fn new(capture: Capture, ledger: Ledger) -> Self {
        let device = capture.identity.device.clone();
        Self { capture: Mutex::new(capture), ledger: Mutex::new(ledger), cancelled: AtomicBool::new(false), device }
    }

    pub(crate) fn poll_uart(&self, port: &mut impl Read, evidence: &mut File,
        quality_verified: impl FnMut() -> bool, lease_valid: impl FnMut() -> bool,
        deadline: Instant) -> Result<usize, String> {
        let mut capture = self.capture.lock().map_err(|_| "STATION.CAPTURE_LOCK")?;
        let result = capture.poll_uart(port, evidence, quality_verified, lease_valid, &self.cancelled, deadline);
        if result.is_err() { self.cancelled.store(true, Ordering::SeqCst); }
        result
    }

    /// Concrete Windows adapter wiring. OS error flags are diagnostic only:
    /// it deliberately rejects readiness until an independently verified
    /// continuity mechanism is integrated for the specific adapter/source.
    #[cfg(windows)]
    pub(crate) fn poll_windows_uart(&self, port: &mut crate::wireless_uart_windows::OwnedUart,
        evidence: &mut File, lease: &ServiceLease,
        deadline: Instant) -> Result<usize, String> {
        let mut capture = self.capture.lock().map_err(|_| "STATION.CAPTURE_LOCK")?;
        if capture.identity.lease_epoch != lease.epoch || port.resource() != lease.resource
            || self.cancelled.load(Ordering::SeqCst)
            || !lease.valid() || Instant::now() >= deadline {
            capture.stop(); self.cancelled.store(true, Ordering::SeqCst);
            return Err("STATION.CAPTURE_STOPPED".into());
        }
        let chunk = match port.read_checked() {
            Ok(chunk) => chunk,
            Err(error) => { capture.stop(); self.cancelled.store(true, Ordering::SeqCst); return Err(error); }
        };
        if self.cancelled.load(Ordering::SeqCst) || !lease.valid() || Instant::now() >= deadline {
            capture.stop(); self.cancelled.store(true, Ordering::SeqCst);
            return Err("STATION.CAPTURE_STOPPED".into());
        }
        let result = capture.ingest(&chunk.bytes, chunk.continuity_verified, evidence, Instant::now())
            .and_then(|()| evidence.sync_data().map_err(|_| "STATION.EVIDENCE_SYNC_FAILED".to_owned()));
        if result.is_err() { capture.stop(); }
        if result.is_err() { self.cancelled.store(true, Ordering::SeqCst); }
        result.map(|()| chunk.bytes.len())
    }

    pub(crate) fn stop(&self) -> Result<(), String> {
        self.cancelled.store(true, Ordering::SeqCst);
        self.capture.lock().map_err(|_| "STATION.CAPTURE_LOCK")?.stop();
        Ok(())
    }

    /// Terminal state remains unresolved until a future trusted recovery path
    /// verifies the physical station. An active record also blocks after a
    /// crash, so failure to write this transition cannot reopen the station.
    pub(crate) fn finish_session_unknown(&self, attempt_id: &str) -> Result<(), String> {
        self.ledger.lock().map_err(|_| "STATION.LEDGER_LOCK".to_owned())?
            .finish_session_unknown(&self.device, attempt_id)
    }

    /// A preliminary check used before opening a device. Actual dispatch must
    /// still recheck under the same capture lock and durable intent protocol.
    pub(crate) fn ready_for_device(&self, lease: &ServiceLease, protection_healthy: bool,
        deadline: Instant) -> Result<(), String> {
        let mut capture = self.capture.lock().map_err(|_| "STATION.CAPTURE_LOCK")?;
        if capture.identity.lease_epoch != lease.epoch || capture.identity.device != lease.resource {
            capture.stop(); return Err("STATION.LEASE_IDENTITY_MISMATCH".into());
        }
        let now = Instant::now();
        if now >= deadline { capture.stop(); return Err("STATION.DISPATCH_TIMEOUT".into()); }
        let id = capture.identity.clone();
        capture.gate.check(&id, now, lease.valid(), self.cancelled.load(Ordering::SeqCst), protection_healthy)
            .map(|_| ()).map_err(|e| format!("STATION.GATE_{e:?}"))
    }

    /// Stop capture immediately and invoke an explicitly supplied station-safe
    /// action. The action must be idempotent and use an independent safety
    /// path: a device operation may still be in flight. Its return
    /// alone is not physical safety feedback; durable state remains unknown.
    pub(crate) fn stop_with_safety(&self, request_id: &str,
        safe_action: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        self.cancelled.store(true, Ordering::SeqCst);
        // Do not wait for an in-flight I/O operation holding the capture lock
        // before requesting the independent safety action. Cancellation is
        // latched first and prevents any later dispatch from starting.
        if let Ok(mut capture) = self.capture.try_lock() { capture.stop(); }
        let record = match self.ledger.try_lock() {
            Ok(mut ledger) => ledger.record_safety_unknown(&self.device, request_id),
            Err(_) => {
                // A dispatch can own the ledger while the hardware call is
                // blocked. Act first, then try to persist uncertainty; never
                // wait for that call before requesting the safety action.
                let action = safe_action();
                let record = self.ledger.lock().map_err(|_| "STATION.LEDGER_LOCK".to_owned())
                    .and_then(|mut ledger| ledger.record_safety_unknown(&self.device, request_id));
                return Err(format!("STATION.SAFETY_UNKNOWN: action={action:?}; ledger={record:?}"));
            }
        };
        if matches!(&record, Err(error) if error == "STATION.SAFETY_REQUEST_ALREADY_SEEN") {
            return Err("STATION.SAFETY_REQUEST_ALREADY_SEEN".into());
        }
        // A storage failure must not prevent the requested safety action.
        let action = safe_action();
        if let Err(error) = record { return Err(format!("STATION.SAFETY_UNKNOWN: {error}")); }
        if let Err(error) = action { return Err(format!("STATION.SAFETY_UNKNOWN: {error}")); }
        Err("STATION.SAFETY_FEEDBACK_UNVERIFIED".into())
    }

    /// This serializes capture invalidation and dispatch inside the process.
    /// Lease and protection must be provided by station-owned mechanisms; the
    /// device driver must provide independent protection during in-flight I/O.
    pub(crate) fn dispatch<T>(&self, deadline: Instant, mut lease_valid: impl FnMut() -> bool,
        mut protection_healthy: impl FnMut() -> bool, command_id: &str,
        operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let mut capture = self.capture.lock().map_err(|_| "STATION.CAPTURE_LOCK")?;
        let mut ledger = self.ledger.lock().map_err(|_| "STATION.LEDGER_LOCK")?;
        let now = Instant::now();
        if now >= deadline { capture.stop(); return Err("STATION.DISPATCH_TIMEOUT".into()); }
        ledger.dispatch(&mut capture, now, lease_valid(), self.cancelled.load(Ordering::SeqCst),
            protection_healthy(), command_id, || {
                // A stop can arrive while the durable intent is being written.
                // Treat that conservatively as unknown rather than reissuing.
                if self.cancelled.load(Ordering::SeqCst) || Instant::now() >= deadline
                    || !lease_valid() || !protection_healthy() {
                    return Err("station state changed before hardware dispatch".into());
                }
                operation()
            })
    }

    pub(crate) fn dispatch_with_service_lease<T>(&self, deadline: Instant, lease: &ServiceLease,
        protection_healthy: impl FnMut() -> bool, command_id: &str,
        operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        let matches = { let capture = self.capture.lock().map_err(|_| "STATION.CAPTURE_LOCK")?;
            capture.identity.lease_epoch == lease.epoch && capture.identity.device == lease.resource };
        if !matches { self.stop()?; return Err("STATION.LEASE_IDENTITY_MISMATCH".into()); }
        self.dispatch(deadline, || lease.valid(), protection_healthy, command_id, operation)
    }
}

#[derive(Debug, serde::Serialize)]
pub(crate) struct UnresolvedDispatch {
    pub command_id: String,
    pub device: String,
    pub state: String,
    pub updated_utc: String,
}

/// Read-only inventory. Counts are exact even when the bounded item lists are
/// truncated. No fields supplied by an API client can assert physical safety.
#[derive(Debug, serde::Serialize)]
pub(crate) struct RecoveryInventory {
    pub device: String,
    pub session_count: u64,
    pub dispatch_count: u64,
    pub safety_count: u64,
    pub sessions: Vec<UnresolvedDispatch>,
    pub dispatches: Vec<UnresolvedDispatch>,
    pub safety_actions: Vec<UnresolvedDispatch>,
    pub truncated: bool,
    pub trusted_ledger_available: bool,
    pub trusted_feedback_available: bool,
    pub recovery_available: bool,
    pub reason: &'static str,
}

impl Ledger {
    pub(crate) fn open(path: &Path) -> Result<Self, String> {
        let db = Connection::open(path).map_err(|e| format!("STATION.LEDGER_OPEN: {e}"))?;
        db.busy_timeout(std::time::Duration::from_secs(2)).map_err(|e| e.to_string())?;
        db.pragma_update(None, "journal_mode", "WAL").map_err(|e| e.to_string())?;
        db.pragma_update(None, "synchronous", "FULL").map_err(|e| e.to_string())?;
        db.execute_batch("CREATE TABLE IF NOT EXISTS station_dispatch (
            command_id TEXT PRIMARY KEY, device TEXT NOT NULL, identity TEXT NOT NULL,
            state TEXT NOT NULL CHECK(state IN ('dispatching','completed','unknown')),
            updated_utc TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS station_dispatch_device ON station_dispatch(device,state);
            CREATE TABLE IF NOT EXISTS station_safety (
                request_id TEXT PRIMARY KEY, device TEXT NOT NULL, state TEXT NOT NULL,
                updated_utc TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS station_safety_device ON station_safety(device,state);
            CREATE TABLE IF NOT EXISTS station_session (
                attempt_id TEXT PRIMARY KEY, device TEXT NOT NULL, identity TEXT NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('active','unknown')),
                updated_utc TEXT NOT NULL);
            CREATE INDEX IF NOT EXISTS station_session_device ON station_session(device,state);")
            .map_err(|e| format!("STATION.LEDGER_SCHEMA: {e}"))?;
        Ok(Self { db })
    }

    /// Write the fence before opening station hardware. Active sessions left
    /// by a crash are indistinguishable from live ones and block new attempts.
    /// No software-only operation here can clear a fence as physically safe.
    pub(crate) fn begin_session(&mut self, identity: &Identity) -> Result<(), String> {
        if identity.attempt.trim().is_empty() || identity.attempt.len() > 256
            || identity.device.trim().is_empty() || identity.device.len() > 256 {
            return Err("STATION.SESSION_IDENTITY_INVALID".into());
        }
        let identity_json = serde_json::to_string(identity).map_err(|e| e.to_string())?;
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("STATION.LEDGER_LOCK: {e}"))?;
        let blocked: bool = tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM station_session WHERE device=?1 AND state IN ('active','unknown'))
             OR EXISTS(SELECT 1 FROM station_dispatch WHERE device=?1 AND state IN ('dispatching','unknown'))
             OR EXISTS(SELECT 1 FROM station_safety WHERE device=?1 AND state='unknown')",
            [&identity.device], |row| row.get(0)).map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
        if blocked { return Err("STATION.RECOVERY_REQUIRED".into()); }
        tx.execute("INSERT INTO station_session(attempt_id,device,identity,state,updated_utc) VALUES(?1,?2,?3,'active',?4)",
            params![identity.attempt, identity.device, identity_json, chrono::Utc::now().to_rfc3339()])
            .map_err(|e| format!("STATION.SESSION_RESERVE: {e}"))?;
        tx.commit().map_err(|e| format!("STATION.LEDGER_COMMIT: {e}"))
    }

    fn finish_session_unknown(&mut self, device: &str, attempt_id: &str) -> Result<(), String> {
        let count = self.db.execute("UPDATE station_session SET state='unknown',updated_utc=?3 WHERE device=?1 AND attempt_id=?2 AND state='active'",
            params![device, attempt_id, chrono::Utc::now().to_rfc3339()])
            .map_err(|e| format!("STATION.SESSION_UPDATE: {e}"))?;
        if count != 1 { return Err("STATION.SESSION_STATE_CHANGED".into()); }
        Ok(())
    }

    pub(crate) fn unresolved_sessions(&self, device: &str) -> Result<Vec<UnresolvedDispatch>, String> {
        if device.trim().is_empty() { return Err("STATION.INVALID_DEVICE".into()); }
        let mut statement = self.db.prepare("SELECT attempt_id,device,state,updated_utc FROM station_session WHERE device=?1 AND state IN ('active','unknown') ORDER BY updated_utc,attempt_id LIMIT 1024")
            .map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
        let rows = statement.query_map([device], |row| Ok(UnresolvedDispatch { command_id: row.get(0)?,
            device: row.get(1)?, state: row.get(2)?, updated_utc: row.get(3)? }))
            .map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("STATION.LEDGER_READ: {e}"))
    }

    /// A single SQLite read transaction gives a consistent inventory. The
    /// trust and recovery flags are hard-coded false until independently
    /// verified storage, identity, authorization and feedback are integrated.
    pub(crate) fn recovery_inventory(&mut self, device: &str) -> Result<RecoveryInventory, String> {
        if device.trim().is_empty() || device.len() > 256 { return Err("STATION.INVALID_DEVICE".into()); }
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Deferred)
            .map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
        fn read_items(tx: &rusqlite::Transaction<'_>, device: &str, count_sql: &str,
            items_sql: &str) -> Result<(u64, Vec<UnresolvedDispatch>), String> {
            let count: u64 = tx.query_row(count_sql, [device], |row| row.get(0))
                .map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
            let mut statement = tx.prepare(items_sql).map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
            let rows = statement.query_map([device], |row| Ok(UnresolvedDispatch { command_id: row.get(0)?,
                device: row.get(1)?, state: row.get(2)?, updated_utc: row.get(3)? }))
                .map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
            let items = rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
            Ok((count, items))
        }
        let (session_count, sessions) = read_items(&tx, device,
            "SELECT COUNT(*) FROM station_session WHERE device=?1 AND state IN ('active','unknown')",
            "SELECT attempt_id,device,state,updated_utc FROM station_session WHERE device=?1 AND state IN ('active','unknown') ORDER BY updated_utc,attempt_id LIMIT 100")?;
        let (dispatch_count, dispatches) = read_items(&tx, device,
            "SELECT COUNT(*) FROM station_dispatch WHERE device=?1 AND state IN ('dispatching','unknown')",
            "SELECT command_id,device,state,updated_utc FROM station_dispatch WHERE device=?1 AND state IN ('dispatching','unknown') ORDER BY updated_utc,command_id LIMIT 100")?;
        let (safety_count, safety_actions) = read_items(&tx, device,
            "SELECT COUNT(*) FROM station_safety WHERE device=?1 AND state='unknown'",
            "SELECT request_id,device,state,updated_utc FROM station_safety WHERE device=?1 AND state='unknown' ORDER BY updated_utc,request_id LIMIT 100")?;
        tx.commit().map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
        Ok(RecoveryInventory { device: device.to_owned(), session_count, dispatch_count, safety_count,
            truncated: session_count > sessions.len() as u64 || dispatch_count > dispatches.len() as u64
                || safety_count > safety_actions.len() as u64,
            sessions, dispatches, safety_actions, trusted_ledger_available: false,
            trusted_feedback_available: false, recovery_available: false,
            reason: "STATION.TRUSTED_RECOVERY_NOT_AVAILABLE" })
    }

    /// Read-only recovery inventory; clearing a record requires a future
    /// trusted physical-readback integration and is intentionally unavailable.
    pub(crate) fn unresolved(&self, device: &str) -> Result<Vec<UnresolvedDispatch>, String> {
        if device.trim().is_empty() { return Err("STATION.INVALID_DEVICE".into()); }
        let mut statement = self.db.prepare("SELECT command_id,device,state,updated_utc FROM station_dispatch WHERE device=?1 AND state IN ('dispatching','unknown') ORDER BY updated_utc,command_id LIMIT 1024")
            .map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
        let rows = statement.query_map([device], |row| Ok(UnresolvedDispatch { command_id: row.get(0)?,
            device: row.get(1)?, state: row.get(2)?, updated_utc: row.get(3)? }))
            .map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("STATION.LEDGER_READ: {e}"))
    }

    pub(crate) fn unresolved_safety(&self, device: &str) -> Result<Vec<UnresolvedDispatch>, String> {
        if device.trim().is_empty() { return Err("STATION.INVALID_DEVICE".into()); }
        let mut statement = self.db.prepare("SELECT request_id,device,state,updated_utc FROM station_safety WHERE device=?1 AND state='unknown' ORDER BY updated_utc,request_id LIMIT 1024")
            .map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
        let rows = statement.query_map([device], |row| Ok(UnresolvedDispatch { command_id: row.get(0)?,
            device: row.get(1)?, state: row.get(2)?, updated_utc: row.get(3)? }))
            .map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
        rows.collect::<Result<Vec<_>, _>>().map_err(|e| format!("STATION.LEDGER_READ: {e}"))
    }

    /// Reserve before touching the device. Any prior unconfirmed action for
    /// the same physical identity blocks even a different run or command ID.
    fn reserve(&mut self, identity: &Identity, command_id: &str) -> Result<(), String> {
        if command_id.trim().is_empty() || command_id.len() > 128 {
            return Err("STATION.INVALID_COMMAND_ID".into());
        }
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("STATION.LEDGER_LOCK: {e}"))?;
        let existing: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM station_dispatch WHERE command_id=?1)",
            [command_id], |row| row.get(0)).map_err(|e| e.to_string())?;
        if existing { return Err("STATION.COMMAND_ALREADY_SEEN".into()); }
        let unresolved: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM station_dispatch WHERE device=?1 AND state IN ('dispatching','unknown'))",
            [&identity.device], |row| row.get(0)).map_err(|e| e.to_string())?;
        if unresolved { return Err("STATION.DEVICE_OUTCOME_UNKNOWN".into()); }
        let unsafe_state: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM station_safety WHERE device=?1 AND state='unknown')",
            [&identity.device], |row| row.get(0)).map_err(|e| e.to_string())?;
        if unsafe_state { return Err("STATION.SAFETY_STATE_UNKNOWN".into()); }
        let identity_json = serde_json::to_string(identity).map_err(|e| e.to_string())?;
        // The durable intent and active session must refer to the exact same
        // station-owned identity. A caller cannot bypass the crash fence by
        // invoking the lower-level ledger dispatch directly.
        let active: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM station_session WHERE attempt_id=?1 AND device=?2 AND identity=?3 AND state='active')",
            params![identity.attempt, identity.device, identity_json], |row| row.get(0))
            .map_err(|e| format!("STATION.LEDGER_READ: {e}"))?;
        if !active { return Err("STATION.ACTIVE_SESSION_REQUIRED".into()); }
        tx.execute("INSERT INTO station_dispatch(command_id,device,identity,state,updated_utc) VALUES(?1,?2,?3,'dispatching',?4)",
            params![command_id, identity.device, identity_json, chrono::Utc::now().to_rfc3339()])
            .map_err(|e| format!("STATION.LEDGER_RESERVE: {e}"))?;
        tx.commit().map_err(|e| format!("STATION.LEDGER_COMMIT: {e}"))
    }

    fn mark(&mut self, command_id: &str, state: &str) -> Result<(), String> {
        let count = self.db.execute("UPDATE station_dispatch SET state=?2,updated_utc=?3 WHERE command_id=?1 AND state='dispatching'",
            params![command_id, state, chrono::Utc::now().to_rfc3339()])
            .map_err(|e| format!("STATION.LEDGER_UPDATE: {e}"))?;
        if count != 1 { return Err("STATION.LEDGER_STATE_CHANGED".into()); }
        Ok(())
    }

    /// Record uncertainty before a safety action. No code in this module can
    /// mark physical safety verified without a trusted feedback integration.
    fn record_safety_unknown(&mut self, device: &str, request_id: &str) -> Result<(), String> {
        if device.trim().is_empty() || request_id.trim().is_empty() || request_id.len() > 128 {
            return Err("STATION.SAFETY_REQUEST_INVALID".into());
        }
        let tx = self.db.transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(|e| format!("STATION.SAFETY_LEDGER: {e}"))?;
        let existing: bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM station_safety WHERE request_id=?1)",
            [request_id], |row| row.get(0)).map_err(|e| format!("STATION.SAFETY_LEDGER: {e}"))?;
        if existing { return Err("STATION.SAFETY_REQUEST_ALREADY_SEEN".into()); }
        tx.execute("INSERT INTO station_safety(request_id,device,state,updated_utc) VALUES(?1,?2,'unknown',?3)",
            params![request_id, device, chrono::Utc::now().to_rfc3339()])
            .map_err(|e| format!("STATION.SAFETY_LEDGER: {e}"))?;
        tx.commit().map_err(|e| format!("STATION.SAFETY_LEDGER: {e}"))
    }

    /// Call only while exclusively owning the capture, the physical session,
    /// and the synchronization shared by invalidation and I²C dispatch. Real
    /// integration must also provide hardware-level protection: a cancellation
    /// arriving after dispatch cannot undo an operation already in flight.
    pub(crate) fn dispatch<T>(&mut self, capture: &mut Capture, now: Instant,
        lease_valid: bool, cancelled: bool, protection_healthy: bool, command_id: &str,
        operation: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
        if capture.failed { return Err("STATION.CAPTURE_CLOSED".into()); }
        capture.gate.check(&capture.identity, now, lease_valid, cancelled, protection_healthy)
            .map_err(|e| format!("STATION.GATE_{e:?}"))?;
        self.reserve(&capture.identity, command_id)?;
        let result = capture.gate.dispatch_once(&capture.identity, now, lease_valid, cancelled,
            protection_healthy, command_id, operation);
        match result {
            Ok(value) => {
                if let Err(error) = self.mark(command_id, "completed") {
                    capture.gate.invalidate(GateError::OutcomeUnknown);
                    return Err(format!("STATION.OUTCOME_UNKNOWN: {error}"));
                }
                Ok(value)
            }
            Err(error) => {
                // Even a failed write to the ledger leaves the earlier durable
                // dispatching intent unresolved, which blocks future attempts.
                let _ = self.mark(command_id, "unknown");
                capture.gate.invalidate(GateError::OutcomeUnknown);
                Err(match error {
                    DispatchError::OutcomeUnknown(e) => format!("STATION.OUTCOME_UNKNOWN: {e}"),
                    other => format!("STATION.DISPATCH_BLOCKED: {other:?}"),
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wireless_gate::ReadinessRule;
    use std::time::Duration;

    fn capture() -> Capture {
        let id = Identity { station: "s".into(), case: "c".into(), run: "r".into(),
            attempt: "a".into(), capture: "cap".into(), device: "device-1".into(),
            lease_epoch: "epoch".into(), config_sha256: "a".repeat(64) };
        let rule = ReadinessRule { version: "fixture".into(), literal: "READY".into(),
            invalidating_literals: vec!["STOPPED".into()], valid_for: Duration::from_secs(30) };
        Capture::new(id.clone(), Gate::new(id, Some(rule), 0).unwrap(), 0)
    }

    #[test]
    fn capture_requires_verified_continuity_and_complete_lines() {
        let mut capture = capture(); let mut evidence = Vec::new(); let now = Instant::now();
        capture.ingest(b"REA", true, &mut evidence, now).unwrap();
        assert_eq!(capture.gate.check(&capture.identity, now, true, false, true).unwrap_err(), GateError::NotReady);
        capture.ingest(b"DY\n", true, &mut evidence, now).unwrap();
        assert!(capture.gate.check(&capture.identity, now, true, false, true).is_ok());
        assert!(capture.ingest(b"anything", false, &mut evidence, now).is_err());
        assert!(capture.ingest(b"READY\n", true, &mut evidence, now).is_err());
    }

    #[test]
    fn unknown_dispatch_blocks_same_device_after_ledger_reopen() {
        let dir = std::env::temp_dir().join(format!("wiparse-station-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("ledger.sqlite3");
        let now = Instant::now(); let mut first = capture();
        first.ingest(b"READY\n", true, &mut Vec::new(), now).unwrap();
        {
            let mut ledger = Ledger::open(&path).unwrap();
            ledger.begin_session(&first.identity).unwrap();
            assert!(ledger.dispatch(&mut first, now, true, false, true, "cmd-1", || Err::<(), _>("lost reply".into())).is_err());
        }
        let mut next = capture();
        next.identity.attempt = "new-attempt".into();
        // Recreate a matching gate for the new attempt.
        let rule = ReadinessRule { version: "fixture".into(), literal: "READY".into(),
            invalidating_literals: vec!["STOPPED".into()], valid_for: Duration::from_secs(30) };
        next.gate = Gate::new(next.identity.clone(), Some(rule), 0).unwrap();
        next.ingest(b"READY\n", true, &mut Vec::new(), now).unwrap();
        let mut ledger = Ledger::open(&path).unwrap(); let mut invoked = false;
        assert_eq!(ledger.unresolved("device-1").unwrap().len(), 1);
        let error = ledger.dispatch(&mut next, now, true, false, true, "cmd-2", || { invoked = true; Ok(()) }).unwrap_err();
        assert!(error.contains("STATION.DEVICE_OUTCOME_UNKNOWN"));
        assert!(!invoked);
        drop(ledger);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn active_session_fence_survives_reopen_and_blocks_new_attempt() {
        let dir = std::env::temp_dir().join(format!("wiparse-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("ledger.sqlite3");
        let identity = capture().identity;
        {
            let mut ledger = Ledger::open(&path).unwrap();
            ledger.begin_session(&identity).unwrap();
        }
        let mut ledger = Ledger::open(&path).unwrap();
        assert_eq!(ledger.unresolved_sessions(&identity.device).unwrap()[0].state, "active");
        let inventory = ledger.recovery_inventory(&identity.device).unwrap();
        assert_eq!(inventory.session_count, 1);
        assert!(!inventory.truncated);
        assert!(!inventory.trusted_ledger_available && !inventory.trusted_feedback_available && !inventory.recovery_available);
        let mut next = identity.clone();
        next.attempt = "next-attempt".into();
        assert_eq!(ledger.begin_session(&next).unwrap_err(), "STATION.RECOVERY_REQUIRED");
        ledger.finish_session_unknown(&identity.device, &identity.attempt).unwrap();
        assert_eq!(ledger.unresolved_sessions(&identity.device).unwrap()[0].state, "unknown");
        assert_eq!(ledger.begin_session(&next).unwrap_err(), "STATION.RECOVERY_REQUIRED");
        drop(ledger);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn dispatch_without_matching_durable_session_is_rejected() {
        let dir = std::env::temp_dir().join(format!("wiparse-no-session-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("ledger.sqlite3");
        let mut ledger = Ledger::open(&path).unwrap();
        let mut source = capture();
        let now = Instant::now();
        source.ingest(b"READY\n", true, &mut Vec::new(), now).unwrap();
        let mut invoked = false;
        assert_eq!(ledger.dispatch(&mut source, now, true, false, true, "cmd", || {
            invoked = true; Ok(())
        }).unwrap_err(), "STATION.ACTIVE_SESSION_REQUIRED");
        assert!(!invoked);
        drop(ledger);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn safety_action_is_recorded_unknown_and_duplicate_is_not_replayed() {
        let dir = std::env::temp_dir().join(format!("wiparse-safety-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&dir).unwrap();
        let path = dir.join("ledger.sqlite3");
        let attempt = StationAttempt::new(capture(), Ledger::open(&path).unwrap());
        let mut calls = 0;
        assert_eq!(attempt.stop_with_safety("stop-1", || { calls += 1; Ok(()) }).unwrap_err(),
            "STATION.SAFETY_FEEDBACK_UNVERIFIED");
        assert_eq!(attempt.stop_with_safety("stop-1", || { calls += 1; Ok(()) }).unwrap_err(),
            "STATION.SAFETY_REQUEST_ALREADY_SEEN");
        assert_eq!(calls, 1);
        drop(attempt);
        let ledger = Ledger::open(&path).unwrap();
        assert_eq!(ledger.unresolved_safety("device-1").unwrap().len(), 1);
        drop(ledger);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
