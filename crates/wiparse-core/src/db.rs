//! SQLite persistence (same schema as Python WiParse).

use crate::metrics::MetricSample;
use crate::paths::project_path;
use chrono::Local;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DbError {
    #[error(transparent)]
    Sqlite(#[from] rusqlite::Error),
    #[error("session not found: {0}")]
    NotFound(i64),
    #[error("{0}")]
    Message(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInfo {
    pub session_id: i64,
    pub session_uuid: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub port: String,
    pub baudrate: i64,
    pub demo_mode: bool,
}

pub fn default_db_path(db_name: &str) -> PathBuf {
    project_path(db_name)
}

pub fn open_db(path: impl AsRef<Path>) -> Result<Connection, DbError> {
    let conn = Connection::open(path)?;
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    init_schema(&conn)?;
    Ok(conn)
}

pub fn init_schema(conn: &Connection) -> Result<(), DbError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS test_sessions (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_uuid TEXT,
            started_at TEXT NOT NULL,
            ended_at TEXT,
            port TEXT,
            baudrate INTEGER,
            demo_mode INTEGER DEFAULT 0
        );
        CREATE TABLE IF NOT EXISTS charging_metrics (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id INTEGER,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
            rel_time REAL, v_in REAL, i_in REAL, v_out REAL, i_out REAL,
            v_bat REAL, i_bat REAL, eff REAL, power REAL, temp REAL, battery REAL
        );
        CREATE TABLE IF NOT EXISTS tx0_logs (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            session_id INTEGER,
            timestamp DATETIME DEFAULT CURRENT_TIMESTAMP,
            rel_time REAL,
            message TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_rel_time ON charging_metrics(rel_time);
        CREATE INDEX IF NOT EXISTS idx_metrics_session ON charging_metrics(session_id);
        CREATE INDEX IF NOT EXISTS idx_tx0_rel_time ON tx0_logs(rel_time);
        CREATE INDEX IF NOT EXISTS idx_logs_session ON tx0_logs(session_id);
        "#,
    )?;
    Ok(())
}

pub fn create_session(
    conn: &Connection,
    port: &str,
    baudrate: u32,
    demo_mode: bool,
) -> Result<SessionInfo, DbError> {
    let started_at = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    let full = Uuid::new_v4().simple().to_string();
    let session_uuid = full[..8].to_ascii_uppercase();
    conn.execute(
        "INSERT INTO test_sessions (session_uuid, started_at, port, baudrate, demo_mode) VALUES (?1,?2,?3,?4,?5)",
        params![session_uuid, started_at, port, baudrate as i64, demo_mode as i64],
    )?;
    let session_id = conn.last_insert_rowid();
    Ok(SessionInfo {
        session_id,
        session_uuid,
        started_at,
        ended_at: None,
        port: port.into(),
        baudrate: baudrate as i64,
        demo_mode,
    })
}

pub fn close_session(conn: &Connection, session_id: i64) -> Result<(), DbError> {
    let ended_at = Local::now().format("%Y-%m-%d %H:%M:%S").to_string();
    conn.execute(
        "UPDATE test_sessions SET ended_at=?1 WHERE id=?2",
        params![ended_at, session_id],
    )?;
    Ok(())
}

pub fn get_session(conn: &Connection, session_id: i64) -> Result<Option<SessionInfo>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, session_uuid, started_at, ended_at, port, baudrate, demo_mode FROM test_sessions WHERE id=?1",
    )?;
    let row = stmt
        .query_row(params![session_id], |r| {
            Ok(SessionInfo {
                session_id: r.get(0)?,
                session_uuid: r.get(1)?,
                started_at: r.get(2)?,
                ended_at: r.get(3)?,
                port: r.get(4)?,
                baudrate: r.get(5)?,
                demo_mode: r.get::<_, i64>(6)? != 0,
            })
        })
        .optional()?;
    Ok(row)
}

pub fn list_sessions(conn: &Connection, limit: usize) -> Result<Vec<SessionInfo>, DbError> {
    let mut stmt = conn.prepare(
        "SELECT id, session_uuid, started_at, ended_at, port, baudrate, demo_mode
         FROM test_sessions ORDER BY id DESC LIMIT ?1",
    )?;
    let rows = stmt.query_map(params![limit as i64], |r| {
        Ok(SessionInfo {
            session_id: r.get(0)?,
            session_uuid: r.get(1)?,
            started_at: r.get(2)?,
            ended_at: r.get(3)?,
            port: r.get(4)?,
            baudrate: r.get(5)?,
            demo_mode: r.get::<_, i64>(6)? != 0,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

pub fn insert_metric(
    conn: &Connection,
    session_id: i64,
    rel_time: f64,
    m: &MetricSample,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO charging_metrics
         (session_id, rel_time, v_in, i_in, v_out, i_out, v_bat, i_bat, eff, power, temp, battery)
         VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)",
        params![
            session_id, rel_time, m.v_in, m.i_in, m.v_out, m.i_out, m.v_bat, m.i_bat, m.eff, m.p,
            m.t as f64, m.b as f64
        ],
    )?;
    Ok(())
}

pub fn insert_log(
    conn: &Connection,
    session_id: i64,
    rel_time: f64,
    message: &str,
) -> Result<(), DbError> {
    conn.execute(
        "INSERT INTO tx0_logs (session_id, rel_time, message) VALUES (?1,?2,?3)",
        params![session_id, rel_time, message],
    )?;
    Ok(())
}

pub fn session_metric_count(conn: &Connection, session_id: i64) -> Result<i64, DbError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM charging_metrics WHERE session_id=?1",
        params![session_id],
        |r| r.get(0),
    )?;
    Ok(n)
}

pub fn session_log_count(conn: &Connection, session_id: i64) -> Result<i64, DbError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM tx0_logs WHERE session_id=?1",
        params![session_id],
        |r| r.get(0),
    )?;
    Ok(n)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::parse_metric_frame;

    #[test]
    fn session_roundtrip() {
        let conn = Connection::open_in_memory().unwrap();
        init_schema(&conn).unwrap();
        let s = create_session(&conn, "COM3", 2_000_000, true).unwrap();
        let m = parse_metric_frame("AA55:9000:1500:8500:1400:4000:3000:45:80:EDED").unwrap();
        insert_metric(&conn, s.session_id, 0.0, &m).unwrap();
        insert_log(&conn, s.session_id, 0.1, "TX0: ASK 02 01 F ").unwrap();
        assert_eq!(session_metric_count(&conn, s.session_id).unwrap(), 1);
        assert_eq!(session_log_count(&conn, s.session_id).unwrap(), 1);
        close_session(&conn, s.session_id).unwrap();
        let got = get_session(&conn, s.session_id).unwrap().unwrap();
        assert!(got.ended_at.is_some());
    }
}
