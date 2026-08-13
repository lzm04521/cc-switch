//! ZCode 会话 provider
//!
//! ZCode 底层基于 opencode，会话库 `~/.zcode/cli/db/db.sqlite` 的
//! `session`/`message`/`part` 三表 schema 是 opencode 的超集（阶段 0 已
//! 通过 PRAGMA table_info 实证），SELECT SQL 可直接复用 opencode provider。
//!
//! 与 opencode 的差异：
//! - db 路径：`~/.zcode/cli/db/db.sqlite`（经 `zcode_config::get_zcode_db_path`）
//! - session id 前缀：`sess_`（双 s），opencode 是 `ses_`（单 s）
//! - ZCode 不支持 CLI resume，`resume_command` 为 None（前端据此置灰恢复按钮）
//! - 无 opencode 旧版 JSON 会话（无 `storage/session/`），仅 SQLite 路径

use std::path::PathBuf;

use rusqlite::Connection;
use serde_json::Value;

use crate::session_manager::{SessionMessage, SessionMeta};

use super::utils::path_basename;

const PROVIDER_ID: &str = "zcode";

fn get_zcode_db_path() -> PathBuf {
    crate::zcode_config::get_zcode_db_path()
}

/// Parse a SQLite source reference in the format `sqlite:<db_path>:<session_id>`.
///
/// Uses `rfind(":sess_")` to split the path from the session ID because the
/// db path itself may contain colons (e.g. `C:\Users\...` on Windows).
/// ZCode session IDs use the `sess_` prefix (double s, unlike opencode's `ses_`).
fn parse_sqlite_source(source: &str) -> Option<(PathBuf, String)> {
    let rest = source.strip_prefix("sqlite:")?;
    let sep = rest.rfind(":sess_")?;
    let db_path = PathBuf::from(&rest[..sep]);
    let session_id = rest[sep + 1..].to_string();
    Some((db_path, session_id))
}

/// Scan sessions from the ZCode SQLite database.
pub fn scan_sessions() -> Vec<SessionMeta> {
    let db_path = get_zcode_db_path();
    if !db_path.exists() {
        return Vec::new();
    }

    let conn = match Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    ) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut stmt = match conn.prepare(
        "SELECT id, title, directory, time_created, time_updated FROM session ORDER BY time_updated DESC",
    ) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };

    let db_display = db_path.display().to_string();

    let iter = match stmt.query_map([], |row| {
        let session_id: String = row.get(0)?;
        let title: String = row.get(1)?;
        let directory: String = row.get(2)?;
        let created: i64 = row.get(3)?;
        let updated: i64 = row.get(4)?;
        Ok((session_id, title, directory, created, updated))
    }) {
        Ok(rows) => rows,
        Err(_) => return Vec::new(),
    };

    let mut sessions = Vec::new();
    for row in iter.flatten() {
        let (session_id, title, directory, created, updated) = row;
        let display_title = if title.is_empty() {
            path_basename(&directory)
        } else {
            Some(title)
        };
        sessions.push(SessionMeta {
            provider_id: PROVIDER_ID.to_string(),
            session_id: session_id.clone(),
            title: display_title.clone(),
            summary: display_title,
            project_dir: if directory.is_empty() {
                None
            } else {
                Some(directory)
            },
            created_at: Some(created),
            last_active_at: Some(updated),
            source_path: Some(format!("sqlite:{db_display}:{session_id}")),
            // ZCode 不支持 CLI resume；前端按 None 置灰恢复按钮
            resume_command: None,
        });
    }
    sessions
}

/// Load messages from the ZCode SQLite database for a given source reference.
/// Joins the `message` and `part` tables in memory to reconstruct full messages.
pub fn load_messages_sqlite(source: &str) -> Result<Vec<SessionMessage>, String> {
    let (db_path, session_id) = parse_sqlite_source(source)
        .ok_or_else(|| format!("Invalid SQLite source reference: {source}"))?;

    let conn = Connection::open_with_flags(
        &db_path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )
    .map_err(|e| format!("Failed to open ZCode database: {e}"))?;

    let mut msg_stmt = conn
        .prepare(
            "SELECT id, time_created, data FROM message WHERE session_id = ?1 ORDER BY time_created ASC",
        )
        .map_err(|e| format!("Failed to prepare message query: {e}"))?;

    let msg_rows = msg_stmt
        .query_map([session_id.as_str()], |row| {
            let id: String = row.get(0)?;
            let ts: i64 = row.get(1)?;
            let data: String = row.get(2)?;
            Ok((id, ts, data))
        })
        .map_err(|e| format!("Failed to query messages: {e}"))?;

    let mut part_stmt = conn
        .prepare(
            "SELECT message_id, data FROM part WHERE session_id = ?1 ORDER BY time_created ASC",
        )
        .map_err(|e| format!("Failed to prepare part query: {e}"))?;

    let part_rows = part_stmt
        .query_map([session_id.as_str()], |row| {
            let message_id: String = row.get(0)?;
            let data: String = row.get(1)?;
            Ok((message_id, data))
        })
        .map_err(|e| format!("Failed to query parts: {e}"))?;

    let mut parts_map: std::collections::HashMap<String, Vec<String>> =
        std::collections::HashMap::new();
    for part in part_rows.flatten() {
        let (message_id, data) = part;
        parts_map.entry(message_id).or_default().push(data);
    }

    let mut messages = Vec::new();
    for row in msg_rows.flatten() {
        let (msg_id, ts, data) = row;
        let msg_value: Value = match serde_json::from_str(&data) {
            Ok(v) => v,
            Err(_) => continue,
        };
        let role = msg_value
            .get("role")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string();

        let mut texts = Vec::new();
        if let Some(parts) = parts_map.get(&msg_id) {
            for part_data in parts {
                let part_value: Value = match serde_json::from_str(part_data) {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                if let Some(text) = extract_part_text(&part_value) {
                    texts.push(text);
                }
            }
        }

        let content = texts.join("\n");
        if content.trim().is_empty() {
            continue;
        }

        messages.push(SessionMessage {
            role,
            content,
            ts: Some(ts),
        });
    }

    Ok(messages)
}

/// Delete a session from the ZCode SQLite database.
pub fn delete_session_sqlite(session_id: &str, source: &str) -> Result<bool, String> {
    let (db_path, ref_session_id) = parse_sqlite_source(source)
        .ok_or_else(|| format!("Invalid SQLite source reference: {source}"))?;
    let db_path = db_path
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize SQLite database path: {e}"))?;
    let expected_db_path = get_zcode_db_path()
        .canonicalize()
        .map_err(|e| format!("Failed to canonicalize expected ZCode database path: {e}"))?;

    if ref_session_id != session_id {
        return Err(format!(
            "ZCode SQLite session ID mismatch: expected {session_id}, found {ref_session_id}"
        ));
    }
    if db_path != expected_db_path {
        return Err("SQLite path does not match expected ZCode database".to_string());
    }

    let conn =
        Connection::open(&db_path).map_err(|e| format!("Failed to open ZCode database: {e}"))?;

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| format!("Failed to begin transaction: {e}"))?;

    tx.execute("DELETE FROM part WHERE session_id = ?1", [session_id])
        .map_err(|e| format!("Failed to delete ZCode parts: {e}"))?;
    tx.execute("DELETE FROM message WHERE session_id = ?1", [session_id])
        .map_err(|e| format!("Failed to delete ZCode messages: {e}"))?;

    let deleted = tx
        .execute("DELETE FROM session WHERE id = ?1", [session_id])
        .map_err(|e| format!("Failed to delete ZCode session: {e}"))?;

    tx.commit()
        .map_err(|e| format!("Failed to commit session deletion: {e}"))?;

    Ok(deleted > 0)
}

fn extract_part_text(part_value: &Value) -> Option<String> {
    match part_value.get("type").and_then(Value::as_str) {
        Some("text") => part_value
            .get("text")
            .and_then(Value::as_str)
            .filter(|t| !t.trim().is_empty())
            .map(|t| t.to_string()),
        Some("tool") => {
            let tool = part_value
                .get("tool")
                .and_then(Value::as_str)
                .unwrap_or("unknown");
            Some(format!("[Tool: {tool}]"))
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use serial_test::serial;

    struct TestHomeGuard(Option<std::ffi::OsString>);
    impl TestHomeGuard {
        fn set(home: &std::path::Path) -> Self {
            let guard = Self(std::env::var_os("CC_SWITCH_TEST_HOME"));
            std::env::set_var("CC_SWITCH_TEST_HOME", home);
            guard
        }
    }
    impl Drop for TestHomeGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(value) => std::env::set_var("CC_SWITCH_TEST_HOME", value),
                None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
            }
        }
    }

    /// 在临时 home 下创建 zcode db（最小 opencode schema，够查询用）
    fn create_test_db(home: &std::path::Path) -> PathBuf {
        let base = home.join(".zcode").join("cli").join("db");
        std::fs::create_dir_all(&base).expect("create db dir");
        let db_path = base.join("db.sqlite");
        let conn = Connection::open(&db_path).expect("open sqlite db");
        conn.execute_batch(
            "
            PRAGMA foreign_keys = ON;
            CREATE TABLE session (
                id TEXT PRIMARY KEY,
                title TEXT NOT NULL,
                directory TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                time_updated INTEGER NOT NULL
            );
            CREATE TABLE message (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                data TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES session(id) ON DELETE CASCADE
            );
            CREATE TABLE part (
                id TEXT PRIMARY KEY,
                session_id TEXT NOT NULL,
                message_id TEXT NOT NULL,
                time_created INTEGER NOT NULL,
                data TEXT NOT NULL,
                FOREIGN KEY(session_id) REFERENCES session(id) ON DELETE CASCADE,
                FOREIGN KEY(message_id) REFERENCES message(id) ON DELETE CASCADE
            );
            ",
        )
        .expect("create sqlite schema");
        db_path
    }

    #[test]
    #[serial]
    fn parse_sqlite_source_accepts_zcode_sess_prefix() {
        let parsed =
            parse_sqlite_source("sqlite:C:/tmp/zcode.db:sess_cb23b701").expect("valid source");
        assert_eq!(parsed.0, PathBuf::from("C:/tmp/zcode.db"));
        assert_eq!(parsed.1, "sess_cb23b701");
    }

    #[test]
    #[serial]
    fn parse_sqlite_source_rejects_opencode_ses_prefix() {
        // opencode 的 `ses_`（单 s）不是 `:sess_` 的子串，必须解析失败——
        // 这保证 opencode 与 zcode 的 source_path 不会互相串provider
        assert!(parse_sqlite_source("sqlite:/tmp/db.sqlite:ses_123").is_none());
        assert!(parse_sqlite_source("/tmp/db.sqlite:sess_123").is_none());
        assert!(parse_sqlite_source("sqlite:/tmp/db.sqlite").is_none());
    }

    #[test]
    #[serial]
    fn scan_sessions_reads_zcode_db_and_sets_no_resume_command() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());
        let db_path = create_test_db(temp.path());

        let conn = Connection::open(&db_path).expect("open");
        conn.execute(
            "INSERT INTO session (id, title, directory, time_created, time_updated) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("sess_1", "", "/tmp/project-a", 1_771_061_953_033_i64, 1_771_061_954_033_i64),
        )
        .expect("insert session 1");
        conn.execute(
            "INSERT INTO session (id, title, directory, time_created, time_updated) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("sess_2", "Named Session", "/tmp/project-b", 1_771_061_950_000_i64, 1_771_061_955_000_i64),
        )
        .expect("insert session 2");
        drop(conn);

        let sessions = scan_sessions();

        assert_eq!(sessions.len(), 2);
        // time_updated DESC 排序
        assert_eq!(sessions[0].session_id, "sess_2");
        assert_eq!(sessions[0].title.as_deref(), Some("Named Session"));
        assert_eq!(sessions[1].session_id, "sess_1");
        // 空 title 回退 directory basename
        assert_eq!(sessions[1].title.as_deref(), Some("project-a"));
        assert_eq!(sessions[1].project_dir.as_deref(), Some("/tmp/project-a"));
        assert_eq!(sessions[1].provider_id, "zcode");
        // zcode 不支持 CLI resume
        assert_eq!(sessions[1].resume_command, None);
        let expected_source = format!("sqlite:{}:sess_1", db_path.display());
        assert_eq!(
            sessions[1].source_path.as_deref(),
            Some(expected_source.as_str())
        );
    }

    #[test]
    #[serial]
    fn load_messages_sqlite_reads_messages_and_parts() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());
        let db_path = create_test_db(temp.path());

        let conn = Connection::open(&db_path).expect("open");
        conn.execute(
            "INSERT INTO session (id, title, directory, time_created, time_updated) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("sess_1", "Session", "/tmp/project-a", 1000_i64, 3000_i64),
        )
        .expect("insert session");
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, ?3, ?4)",
            ("msg_1", "sess_1", 1000_i64, r#"{"role":"user"}"#),
        )
        .expect("insert message 1");
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, ?3, ?4)",
            ("msg_2", "sess_1", 2000_i64, r#"{"role":"assistant"}"#),
        )
        .expect("insert message 2");
        conn.execute(
            "INSERT INTO part (id, session_id, message_id, time_created, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("prt_1", "sess_1", "msg_1", 1000_i64, r#"{"type":"text","text":"Hello"}"#),
        )
        .expect("insert part 1");
        conn.execute(
            "INSERT INTO part (id, session_id, message_id, time_created, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("prt_2", "sess_1", "msg_2", 2000_i64, r#"{"type":"tool","tool":"bash"}"#),
        )
        .expect("insert part 2");
        conn.execute(
            "INSERT INTO part (id, session_id, message_id, time_created, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("prt_3", "sess_1", "msg_2", 2001_i64, r#"{"type":"text","text":"Done"}"#),
        )
        .expect("insert part 3");
        drop(conn);

        let source = format!("sqlite:{}:sess_1", db_path.display());
        let messages = load_messages_sqlite(&source).expect("load sqlite messages");

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0].role, "user");
        assert_eq!(messages[0].content, "Hello");
        assert_eq!(messages[1].role, "assistant");
        assert_eq!(messages[1].content, "[Tool: bash]\nDone");
    }

    #[test]
    #[serial]
    fn delete_session_sqlite_removes_session() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());
        let db_path = create_test_db(temp.path());

        let conn = Connection::open(&db_path).expect("open");
        conn.execute(
            "INSERT INTO session (id, title, directory, time_created, time_updated) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("sess_1", "Session", "/tmp/project-a", 1000_i64, 3000_i64),
        )
        .expect("insert session");
        conn.execute(
            "INSERT INTO message (id, session_id, time_created, data) VALUES (?1, ?2, ?3, ?4)",
            ("msg_1", "sess_1", 1000_i64, r#"{"role":"user"}"#),
        )
        .expect("insert message");
        conn.execute(
            "INSERT INTO part (id, session_id, message_id, time_created, data) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("prt_1", "sess_1", "msg_1", 1000_i64, r#"{"type":"text","text":"Hello"}"#),
        )
        .expect("insert part");
        drop(conn);

        let source = format!("sqlite:{}:sess_1", db_path.display());
        let deleted = delete_session_sqlite("sess_1", &source).expect("delete sqlite session");
        assert!(deleted);

        let conn = Connection::open(&db_path).expect("re-open sqlite db");
        let remaining_sessions: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM session WHERE id = 'sess_1'",
                [],
                |row| row.get(0),
            )
            .expect("count sessions");
        let remaining_messages: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM message WHERE session_id = 'sess_1'",
                [],
                |row| row.get(0),
            )
            .expect("count messages");
        let remaining_parts: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM part WHERE session_id = 'sess_1'",
                [],
                |row| row.get(0),
            )
            .expect("count parts");

        assert_eq!(remaining_sessions, 0);
        assert_eq!(remaining_messages, 0);
        assert_eq!(remaining_parts, 0);
    }

    #[test]
    #[serial]
    fn delete_session_sqlite_rejects_foreign_db_path() {
        let temp = tempfile::tempdir().expect("tempdir");
        let _guard = TestHomeGuard::set(temp.path());
        let expected_db_path = create_test_db(temp.path());
        Connection::open(&expected_db_path).expect("create expected sqlite db");

        let foreign_path = temp.path().join("foreign.db");
        let conn = Connection::open(&foreign_path).expect("open sqlite db");
        conn.execute_batch(
            "
            CREATE TABLE session (id TEXT PRIMARY KEY, title TEXT NOT NULL, directory TEXT NOT NULL, time_created INTEGER NOT NULL, time_updated INTEGER NOT NULL);
            CREATE TABLE message (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, time_created INTEGER NOT NULL, data TEXT NOT NULL);
            CREATE TABLE part (id TEXT PRIMARY KEY, session_id TEXT NOT NULL, message_id TEXT NOT NULL, time_created INTEGER NOT NULL, data TEXT NOT NULL);
            ",
        )
        .expect("create schema");
        conn.execute(
            "INSERT INTO session (id, title, directory, time_created, time_updated) VALUES (?1, ?2, ?3, ?4, ?5)",
            ("sess_1", "Session", "/tmp/project", 1000_i64, 3000_i64),
        )
        .expect("insert session");
        drop(conn);

        let source = format!("sqlite:{}:sess_1", foreign_path.display());
        let err = delete_session_sqlite("sess_1", &source).expect_err("should reject foreign db");
        assert!(err.contains("expected ZCode database"));
    }
}
