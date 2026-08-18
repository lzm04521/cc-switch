use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use serde_json::json;
use tauri::{AppHandle, Emitter};
use tokio::sync::mpsc::error::TrySendError;
use tokio::sync::mpsc::{channel, Receiver, Sender};

use crate::error::AppError;
use crate::services::sync_protocol::should_trigger_auto_sync_for_table;
use crate::services::webdav_sync as webdav_sync_service;
use crate::settings::{self, WebDavSyncSettings};

const AUTO_SYNC_DEBOUNCE_MS: u64 = 1000;
pub(crate) const MAX_AUTO_SYNC_WAIT_MS: u64 = 10_000;

static DB_CHANGE_TX: OnceLock<Sender<String>> = OnceLock::new();
static AUTO_SYNC_SUPPRESS_DEPTH: AtomicUsize = AtomicUsize::new(0);

pub(crate) struct AutoSyncSuppressionGuard;

impl AutoSyncSuppressionGuard {
    pub fn new() -> Self {
        AUTO_SYNC_SUPPRESS_DEPTH.fetch_add(1, Ordering::SeqCst);
        Self
    }
}

impl Drop for AutoSyncSuppressionGuard {
    fn drop(&mut self) {
        let _ =
            AUTO_SYNC_SUPPRESS_DEPTH.fetch_update(Ordering::SeqCst, Ordering::SeqCst, |value| {
                Some(value.saturating_sub(1))
            });
    }
}

pub(crate) fn is_auto_sync_suppressed() -> bool {
    AUTO_SYNC_SUPPRESS_DEPTH.load(Ordering::SeqCst) > 0
}

pub fn should_trigger_for_table(table: &str) -> bool {
    should_trigger_auto_sync_for_table(table)
}

pub(crate) fn enqueue_change_signal(tx: &Sender<String>, table: &str) -> bool {
    match tx.try_send(table.to_string()) {
        Ok(()) => true,
        Err(TrySendError::Full(_)) | Err(TrySendError::Closed(_)) => false,
    }
}

pub(crate) fn auto_sync_wait_duration(started_at: Instant, now: Instant) -> Option<Duration> {
    let max_wait = Duration::from_millis(MAX_AUTO_SYNC_WAIT_MS);
    let debounce = Duration::from_millis(AUTO_SYNC_DEBOUNCE_MS);
    let elapsed = now.saturating_duration_since(started_at);
    if elapsed >= max_wait {
        return None;
    }
    Some(debounce.min(max_wait - elapsed))
}

fn should_run_auto_sync(settings: Option<&WebDavSyncSettings>) -> bool {
    let Some(sync) = settings else {
        return false;
    };
    sync.enabled && sync.auto_sync
}

fn persist_auto_sync_error(settings: &mut WebDavSyncSettings, error: &AppError) {
    settings.status.last_error = Some(error.to_string());
    settings.status.last_error_source = Some("auto".to_string());
    let _ = settings::update_webdav_sync_status(settings.status.clone());
}

fn emit_auto_sync_status_updated(app: &AppHandle, status: &str, error: Option<&str>) {
    let payload = match error {
        Some(message) => json!({
            "source": "auto",
            "status": status,
            "error": message,
        }),
        None => json!({
            "source": "auto",
            "status": status,
        }),
    };

    if let Err(err) = app.emit("webdav-sync-status-updated", payload) {
        log::debug!("[WebDAV] failed to emit sync status update event: {err}");
    }
}

async fn run_auto_sync_upload(
    db: &crate::database::Database,
    app: &AppHandle,
) -> Result<(), AppError> {
    let mut settings = settings::get_webdav_sync_settings();
    if !should_run_auto_sync(settings.as_ref()) {
        return Ok(());
    }

    let mut sync_settings = match settings.take() {
        Some(value) => value,
        None => return Ok(()),
    };

    let result = webdav_sync_service::run_with_sync_lock(webdav_sync_service::upload(
        db,
        &mut sync_settings,
    ))
    .await;
    match result {
        Ok(_) => {
            emit_auto_sync_status_updated(app, "success", None);
            Ok(())
        }
        Err(err) => {
            persist_auto_sync_error(&mut sync_settings, &err);
            emit_auto_sync_status_updated(app, "error", Some(&err.to_string()));
            Err(err)
        }
    }
}

pub fn notify_db_changed(table: &str) {
    if is_auto_sync_suppressed() {
        return;
    }
    if !should_trigger_for_table(table) {
        return;
    }
    let Some(tx) = DB_CHANGE_TX.get() else {
        return;
    };
    let _ = enqueue_change_signal(tx, table);
}

pub fn start_worker(db: Arc<crate::database::Database>, app: tauri::AppHandle) {
    if DB_CHANGE_TX.get().is_some() {
        return;
    }

    // Buffer size 1 is enough: we only need "dirty" signals, not every event.
    let (tx, rx) = channel::<String>(1);
    if DB_CHANGE_TX.set(tx).is_err() {
        return;
    }

    // 启动延迟（分钟）：启动后窗口内暂停自动备份，等待网络（如 ZeroTier）就绪。
    // 0 表示不启用；启动时读一次，运行中修改需重启生效。
    let startup_delay_minutes = settings::get_webdav_sync_settings()
        .map(|s| s.startup_delay_minutes)
        .unwrap_or(0);

    tauri::async_runtime::spawn(async move {
        run_worker_loop(db, rx, app, startup_delay_minutes).await;
    });
}

/// 启动延迟窗口：在 `delay` 时长内持续接收变更信号但不上传，
/// 返回窗口内是否收到过变更（用于窗口结束时决定是否补传一次）。
async fn drain_startup_window(rx: &mut Receiver<String>, delay: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + delay;
    let mut had_change = false;
    loop {
        match tokio::time::timeout_at(deadline, rx.recv()).await {
            Ok(Some(_)) => had_change = true,
            Ok(None) | Err(_) => return had_change,
        }
    }
}

async fn run_worker_loop(
    db: Arc<crate::database::Database>,
    mut rx: Receiver<String>,
    app: tauri::AppHandle,
    startup_delay_minutes: u32,
) {
    // 启动延迟窗口：等待网络就绪（如 ZeroTier），窗口内变更合并为窗口结束时的一次补传。
    if startup_delay_minutes > 0 {
        let delay = Duration::from_secs(u64::from(startup_delay_minutes) * 60);
        let had_change = drain_startup_window(&mut rx, delay).await;
        if had_change {
            log::info!(
                "[WebDAV][AutoSync] Startup delay {startup_delay_minutes}min elapsed, flushing pending backup"
            );
            if let Err(err) = run_auto_sync_upload(&db, &app).await {
                log::warn!("[WebDAV][AutoSync] Startup-delay flush failed: {err}");
            }
        }
    }

    while let Some(first_table) = rx.recv().await {
        let started_at = Instant::now();
        let mut merged_count = 1usize;

        while let Some(wait_for) = auto_sync_wait_duration(started_at, Instant::now()) {
            let timeout = tokio::time::timeout(wait_for, rx.recv()).await;

            match timeout {
                Ok(Some(_)) => merged_count += 1,
                Ok(None) => return,
                Err(_) => break,
            }
        }

        log::debug!(
            "[WebDAV][AutoSync] Triggered by table={first_table}, merged_changes={merged_count}"
        );

        if let Err(err) = run_auto_sync_upload(&db, &app).await {
            log::warn!("[WebDAV][AutoSync] Upload failed: {err}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        auto_sync_wait_duration, drain_startup_window, enqueue_change_signal,
        is_auto_sync_suppressed, should_run_auto_sync, should_trigger_for_table,
        AutoSyncSuppressionGuard, MAX_AUTO_SYNC_WAIT_MS,
    };
    use crate::settings::WebDavSyncSettings;
    use serial_test::serial;
    use std::time::{Duration, Instant};
    use tokio::sync::mpsc::channel;

    #[test]
    fn should_trigger_sync_for_config_tables_only() {
        assert!(should_trigger_for_table("providers"));
        assert!(should_trigger_for_table("profiles"));
        assert!(should_trigger_for_table("settings"));
        assert!(!should_trigger_for_table("proxy_request_logs"));
        assert!(!should_trigger_for_table("provider_health"));
    }

    #[test]
    #[serial]
    fn suppression_guard_enables_and_restores_state() {
        assert!(!is_auto_sync_suppressed());
        {
            let _guard = AutoSyncSuppressionGuard::new();
            assert!(is_auto_sync_suppressed());
        }
        assert!(!is_auto_sync_suppressed());
    }

    #[test]
    fn max_wait_caps_flush_latency_for_continuous_events() {
        let started = Instant::now();
        let later = started + Duration::from_millis(MAX_AUTO_SYNC_WAIT_MS + 1);
        assert!(auto_sync_wait_duration(started, later).is_none());
    }

    #[tokio::test]
    async fn enqueue_change_signal_drops_when_channel_is_full() {
        let (tx, _rx) = channel::<String>(1);
        assert!(enqueue_change_signal(&tx, "providers"));
        assert!(!enqueue_change_signal(&tx, "providers"));
    }

    #[tokio::test]
    async fn drain_startup_window_returns_false_when_no_change() {
        let (_tx, mut rx) = channel::<String>(1);
        let had = drain_startup_window(&mut rx, Duration::from_millis(30)).await;
        assert!(!had);
    }

    #[tokio::test]
    async fn drain_startup_window_returns_true_on_change() {
        let (tx, mut rx) = channel::<String>(1);
        tx.try_send("providers".to_string()).unwrap();
        let had = drain_startup_window(&mut rx, Duration::from_millis(30)).await;
        assert!(had);
    }

    #[test]
    fn should_run_auto_sync_requires_enabled_and_auto_sync_flag() {
        assert!(!should_run_auto_sync(None));

        let disabled = WebDavSyncSettings {
            enabled: false,
            auto_sync: true,
            ..WebDavSyncSettings::default()
        };
        assert!(!should_run_auto_sync(Some(&disabled)));

        let auto_sync_off = WebDavSyncSettings {
            enabled: true,
            auto_sync: false,
            ..WebDavSyncSettings::default()
        };
        assert!(!should_run_auto_sync(Some(&auto_sync_off)));

        let enabled = WebDavSyncSettings {
            enabled: true,
            auto_sync: true,
            ..WebDavSyncSettings::default()
        };
        assert!(should_run_auto_sync(Some(&enabled)));
    }

    #[test]
    fn service_layer_does_not_depend_on_commands_layer() {
        let source = include_str!("webdav_auto_sync.rs");
        let needle = ["crate", "commands", ""].join("::");
        assert!(
            !source.contains(&needle),
            "services layer should not depend on commands layer"
        );
    }
}
