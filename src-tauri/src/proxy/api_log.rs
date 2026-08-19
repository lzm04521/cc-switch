//! API 报文记录（调试用途）
//!
//! 与 `enable_logging`（事件级运行日志）互补：本模块在打开开关后，把代理
//! 链路的三段原始报文完整落盘，用于排查"模型映射/优化器到底改了什么"、
//! "上游回了什么"这类需要看原文的问题：
//! - `received`：客户端发给本地代理的请求
//! - `forwards[]`：每次上游尝试（故障转移/整流重试各记一条）的出站请求与上游响应
//! - `final`：代理发回客户端的最终回复（格式转换场景与上游原文不同）
//!
//! 设计约束：
//! - 报文可能很大（含完整对话上下文），必须经 channel 由专职线程写盘，
//!   绝不阻塞请求热路径；channel 满则丢弃并告警。
//! - 报文含敏感凭据，header 一律脱敏；URL 复用 forwarder 已脱敏的
//!   `target_for_log`，不在本模块重复处理。
//! - 目录按请求数量滚动清理（默认 200 个文件），磁盘占用可控。
//! - `ApiLogCapture` 通过 `Drop` 兜底落盘：成功、失败、客户端断连、流中断
//!   任何出口都保证已捕获的现场写盘（幂等）。

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, mpsc};

use serde::{Deserialize, Serialize};

/// settings 表中的配置键（JSON: ApiLogConfig）
pub const API_LOG_CONFIG_KEY: &str = "api_log_config";
/// 目录内保留的最大请求数（文件），超出按最旧删除
pub const MAX_LOG_FILES: usize = 200;

// ============================================================================
// 配置与全局开关
// ============================================================================

/// API 报文记录配置（存储于 settings 表 `api_log_config` 键）
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ApiLogConfig {
    /// 总开关：是否记录 API 报文（默认关闭——报文含完整对话上下文与代码隐私）
    #[serde(default)]
    pub enabled: bool,
}

static ENABLED: AtomicBool = AtomicBool::new(false);

/// 更新全局开关（由设置命令与代理启动时调用，运行中切换即时生效）
pub fn set_enabled(enabled: bool) {
    ENABLED.store(enabled, Ordering::Release);
}

pub fn is_enabled() -> bool {
    ENABLED.load(Ordering::Acquire)
}

/// 报文落盘目录：与应用运行日志同级的 `api_logs/` 子目录
pub fn api_log_dir() -> PathBuf {
    crate::panic_hook::get_log_dir().join("api_logs")
}

// ============================================================================
// 落盘文件结构
// ============================================================================

fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339_opts(chrono::SecondsFormat::Millis, false)
}

/// 一条 HTTP 报文（请求或响应体）
#[derive(Debug, Clone, Serialize)]
pub struct MessageLog {
    pub timestamp: String,
    pub headers: BTreeMap<String, String>,
    /// 报文原文（UTF-8 lossy；JSON/SSE 均为文本）
    pub body: String,
}

/// 客户端发来的请求（received 段）
#[derive(Debug, Clone, Serialize)]
pub struct ReceivedLog {
    pub timestamp: String,
    pub method: String,
    pub endpoint: String,
    pub headers: BTreeMap<String, String>,
    pub body: String,
}

/// 单次上游尝试：出站请求 + 上游响应
#[derive(Debug, Clone, Serialize)]
pub struct ForwardLog {
    /// 第几次上游尝试（1 起）
    pub attempt: usize,
    pub provider_id: String,
    pub provider_name: String,
    /// 已脱敏的请求目标 URL
    pub url: String,
    pub method: String,
    pub request: MessageLog,
    /// 发送失败时 response 为空壳（status = None）
    pub response: ResponseLog,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct ResponseLog {
    pub timestamp: Option<String>,
    pub status: Option<u16>,
    pub headers: BTreeMap<String, String>,
    /// 上游响应体（流式响应在流结束后回填；转换场景上游原文不可得时为空）
    pub body: String,
    pub is_stream: bool,
}

/// 发回客户端的最终回复
#[derive(Debug, Clone, Serialize)]
pub struct FinalLog {
    pub timestamp: String,
    pub status: Option<u16>,
    pub body: String,
    /// 说明：透传时与最后一条 forward 的响应体相同（不重复存储）；转换场景为转换器输出
    pub note: String,
}

/// 每请求一个落盘文件的内容
#[derive(Debug, Clone, Serialize)]
pub struct ApiLogFile {
    pub request_id: String,
    pub app_type: String,
    pub model: String,
    pub created_at: String,
    pub received: Option<ReceivedLog>,
    pub forwards: Vec<ForwardLog>,
    #[serde(rename = "final")]
    pub final_response: Option<FinalLog>,
}

// ============================================================================
// 请求级捕获器
// ============================================================================

/// 流式响应 tee 的记录目标
///
/// 同一个透传流包装器（create_logged_passthrough_stream）既用于纯透传（字节
/// 即上游响应体），也用于转换器输出（字节是发回客户端的最终回复），两者
/// 记入的文件段落不同。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TeeTarget {
    /// 纯透传：字节回填到 forwards[-1].response.body，final 标注与之一致
    UpstreamPassthrough,
    /// 转换器输出：字节记入 final（此路径上游原文在转换器内部，不可得）
    FinalOutput,
}

struct CaptureInner {
    request_id: String,
    app_type: String,
    model: String,
    received: Option<ReceivedLog>,
    forwards: Vec<ForwardLog>,
    final_response: Option<FinalLog>,
    flushed: bool,
}

/// 单请求报文捕获器，随 `RequestContext` / forwarder / 响应流克隆流转，
/// 最后一个引用 drop 时兜底落盘。
#[derive(Clone)]
pub struct ApiLogCapture {
    inner: Arc<Mutex<CaptureInner>>,
}

impl Drop for CaptureInner {
    fn drop(&mut self) {
        flush_locked(self);
    }
}

impl ApiLogCapture {
    pub fn new(request_id: String, app_type: String, model: String) -> Self {
        Self {
            inner: Arc::new(Mutex::new(CaptureInner {
                request_id,
                app_type,
                model,
                received: None,
                forwards: Vec::new(),
                final_response: None,
                flushed: false,
            })),
        }
    }

    /// 请求入口：客户端 → 本地代理
    pub fn record_received(
        &self,
        method: &str,
        endpoint: &str,
        headers: &http::HeaderMap,
        body: &[u8],
    ) {
        let mut guard = self.inner.lock().unwrap();
        guard.received = Some(ReceivedLog {
            timestamp: now_rfc3339(),
            method: method.to_string(),
            endpoint: endpoint.to_string(),
            headers: sanitize_headers(headers),
            body: lossy(body),
        });
    }

    /// forwarder 发送前：本地代理 → 上游（每次尝试追加一条）
    pub fn record_forward_request(
        &self,
        provider_id: &str,
        provider_name: &str,
        url: &str,
        method: &str,
        headers: &http::HeaderMap,
        body: &[u8],
    ) {
        let mut guard = self.inner.lock().unwrap();
        let attempt = guard.forwards.len() + 1;
        guard.forwards.push(ForwardLog {
            attempt,
            provider_id: provider_id.to_string(),
            provider_name: provider_name.to_string(),
            url: url.to_string(),
            method: method.to_string(),
            request: MessageLog {
                timestamp: now_rfc3339(),
                headers: sanitize_headers(headers),
                body: lossy(body),
            },
            response: ResponseLog::default(),
        });
    }

    /// forwarder 收到上游响应头后（发送失败则不调用，response 保持空壳）
    pub fn record_forward_response_head(
        &self,
        status: u16,
        headers: &http::HeaderMap,
        is_stream: bool,
    ) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(entry) = guard.forwards.last_mut() {
            entry.response = ResponseLog {
                timestamp: Some(now_rfc3339()),
                status: Some(status),
                headers: sanitize_headers(headers),
                body: String::new(),
                is_stream,
            };
        }
    }

    /// 响应体到位后回填最后一条上游尝试（非流式整包；流式在流结束时累积回填）
    pub fn record_forward_response_body(&self, body: &[u8]) {
        let mut guard = self.inner.lock().unwrap();
        if let Some(entry) = guard.forwards.last_mut() {
            entry.response.body = lossy(body);
        }
    }

    /// 发回客户端的最终回复（显式记录状态与说明；透传场景 body 传空 + note 说明）
    pub fn record_final(&self, status: Option<u16>, body: &[u8], note: &str) {
        let mut guard = self.inner.lock().unwrap();
        guard.final_response = Some(FinalLog {
            timestamp: now_rfc3339(),
            status,
            body: lossy(body),
            note: note.to_string(),
        });
    }

    /// 立即落盘（幂等；正常响应路径主动调用，Drop 兜底会跳过已落盘的）
    pub fn flush(&self) {
        let mut guard = self.inner.lock().unwrap();
        flush_locked(&mut guard);
    }
}

fn flush_locked(inner: &mut CaptureInner) {
    if inner.flushed {
        return;
    }
    inner.flushed = true;

    let file = ApiLogFile {
        request_id: inner.request_id.clone(),
        app_type: inner.app_type.clone(),
        model: inner.model.clone(),
        created_at: now_rfc3339(),
        received: inner.received.clone(),
        forwards: inner.forwards.clone(),
        final_response: inner.final_response.clone(),
    };

    let content = serde_json::to_string_pretty(&file).unwrap_or_default();
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f");
    let short_id = file
        .request_id
        .chars()
        .take(8)
        .collect::<String>()
        .to_ascii_lowercase();
    let file_name = format!("{timestamp}_{short_id}.json");
    if let Some(sender) = ensure_writer() {
        // 有界 channel 满说明写盘严重滞后（磁盘极慢），丢弃本条而不是阻塞
        // 请求线程——Drop 兜底可能在任意请求线程上触发。
        let outcome = sender.try_send((file_name.clone(), content));
        if outcome.is_err() {
            log::warn!("[ApiLog] 报文记录入队失败（队列满或写盘线程关闭），已丢弃: {file_name}");
        }
    }
}

// ============================================================================
// 写盘线程与目录清理
// ============================================================================

type WriteJob = (String, String);

static WRITER: OnceLock<mpsc::SyncSender<WriteJob>> = OnceLock::new();

/// 启动（或复用）专职写盘线程。channel 满时 `send` 阻塞会卡住请求线程，
/// 因此用有界同步 channel + `try_send`，满则丢弃。
fn ensure_writer() -> Option<mpsc::SyncSender<WriteJob>> {
    if let Some(sender) = WRITER.get() {
        return Some(sender.clone());
    }

    let (sender, receiver) = mpsc::sync_channel::<WriteJob>(256);
    let dir = api_log_dir();
    if let Err(e) = std::fs::create_dir_all(&dir) {
        log::warn!("[ApiLog] 创建报文记录目录失败，功能停用: {dir:?} - {e}");
        return None;
    }

    std::thread::Builder::new()
        .name("api-log-writer".into())
        .spawn(move || {
            for (file_name, content) in receiver {
                let path = dir.join(&file_name);
                if let Err(e) = std::fs::write(&path, content) {
                    log::warn!("[ApiLog] 写入报文文件失败: {path:?} - {e}");
                    continue;
                }
                if let Err(e) = enforce_retention(&dir, MAX_LOG_FILES) {
                    log::warn!("[ApiLog] 清理过期报文文件失败: {dir:?} - {e}");
                }
            }
        })
        .ok()?;

    let _ = WRITER.set(sender.clone());
    Some(sender)
}

/// 按文件名字典序（时间戳前缀保证与时间序一致）保留最新 max_files 个
pub(crate) fn enforce_retention(dir: &std::path::Path, max_files: usize) -> std::io::Result<()> {
    let mut files: Vec<String> = std::fs::read_dir(dir)?
        .filter_map(|entry| {
            let entry = entry.ok()?;
            let name = entry.file_name().to_string_lossy().to_string();
            (name.ends_with(".json")).then_some(name)
        })
        .collect();
    if files.len() <= max_files {
        return Ok(());
    }
    files.sort();
    let excess = files.len() - max_files;
    for name in files.into_iter().take(excess) {
        std::fs::remove_file(dir.join(name))?;
    }
    Ok(())
}

// ============================================================================
// 脱敏与工具
// ============================================================================

/// 敏感请求/响应头：值整体打码（保留键名便于诊断）
const SENSITIVE_HEADERS: &[&str] = &[
    "authorization",
    "proxy-authorization",
    "x-api-key",
    "api-key",
    "x-goog-api-key",
    "cookie",
    "set-cookie",
    "openai-session-id",
];

pub fn sanitize_headers(headers: &http::HeaderMap) -> BTreeMap<String, String> {
    let mut map: BTreeMap<String, String> = BTreeMap::new();
    for (name, value) in headers.iter() {
        let key = name.as_str().to_ascii_lowercase();
        let text = value.to_str().unwrap_or_default();
        let masked = SENSITIVE_HEADERS
            .iter()
            .any(|s| key.eq_ignore_ascii_case(s));
        let rendered = if masked {
            "***".to_string()
        } else {
            text.to_string()
        };
        // 同名多值头（如 set-cookie、多个 x-forwarded-for）拼接保留
        match map.entry(key) {
            std::collections::btree_map::Entry::Occupied(mut entry) => {
                let existing = entry.get_mut();
                existing.push_str(", ");
                existing.push_str(&rendered);
            }
            std::collections::btree_map::Entry::Vacant(entry) => {
                entry.insert(rendered);
            }
        }
    }
    map
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

// ============================================================================
// 测试
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn header_map(entries: &[(&str, &str)]) -> http::HeaderMap {
        let mut map = http::HeaderMap::new();
        for (name, value) in entries {
            map.insert(
                http::HeaderName::from_bytes(name.as_bytes()).unwrap(),
                http::HeaderValue::from_str(value).unwrap(),
            );
        }
        map
    }

    #[test]
    fn sanitize_headers_masks_sensitive_and_keeps_others() {
        let map = header_map(&[
            ("Authorization", "Bearer sk-secret"),
            ("X-Api-Key", "abc123"),
            ("Content-Type", "application/json"),
            ("Anthropic-Version", "2023-06-01"),
        ]);
        let sanitized = sanitize_headers(&map);
        assert_eq!(sanitized.get("authorization").unwrap(), "***");
        assert_eq!(sanitized.get("x-api-key").unwrap(), "***");
        assert_eq!(
            sanitized.get("content-type").unwrap(),
            "application/json"
        );
        assert_eq!(sanitized.get("anthropic-version").unwrap(), "2023-06-01");
    }

    #[test]
    fn sanitize_headers_joins_multi_value() {
        let mut map = http::HeaderMap::new();
        map.append(
            http::HeaderName::from_static("x-forwarded-for"),
            http::HeaderValue::from_static("1.1.1.1"),
        );
        map.append(
            http::HeaderName::from_static("x-forwarded-for"),
            http::HeaderValue::from_static("2.2.2.2"),
        );
        let sanitized = sanitize_headers(&map);
        assert_eq!(
            sanitized.get("x-forwarded-for").unwrap(),
            "1.1.1.1, 2.2.2.2"
        );
    }

    #[test]
    fn api_log_config_defaults_to_disabled() {
        let config: ApiLogConfig = serde_json::from_str("{}").unwrap();
        assert!(!config.enabled);
    }

    #[test]
    fn capture_records_full_request_lifecycle() {
        let capture = ApiLogCapture::new(
            "req-1234567890".to_string(),
            "claude".to_string(),
            "claude-sonnet-4".to_string(),
        );
        capture.record_received(
            "POST",
            "/v1/messages",
            &header_map(&[("Authorization", "Bearer sk-live")]),
            br#"{"model":"claude-sonnet-4","stream":true}"#,
        );
        capture.record_forward_request(
            "provider-1",
            "中转 A",
            "https://gw.example.com/v1/messages",
            "POST",
            &header_map(&[("X-Api-Key", "upstream-key")]),
            br#"{"model":"gpt-5.6","stream":true}"#,
        );
        capture.record_forward_response_head(
            200,
            &header_map(&[("Content-Type", "text/event-stream")]),
            true,
        );
        capture.record_forward_response_body(b"data: {\"type\":\"message_start\"}\n\n");
        capture.record_final(None, b"", "passthrough: body identical to last forward response");

        let inner = capture.inner.lock().unwrap();
        let received = inner.received.as_ref().unwrap();
        assert_eq!(received.method, "POST");
        assert_eq!(received.endpoint, "/v1/messages");
        assert!(received.body.contains("claude-sonnet-4"));
        assert_eq!(received.headers.get("authorization").unwrap(), "***");

        assert_eq!(inner.forwards.len(), 1);
        let forward = &inner.forwards[0];
        assert_eq!(forward.attempt, 1);
        assert_eq!(forward.provider_name, "中转 A");
        assert!(forward.request.body.contains("gpt-5.6"));
        assert_eq!(forward.request.headers.get("x-api-key").unwrap(), "***");
        assert_eq!(forward.response.status, Some(200));
        assert!(forward.response.body.contains("message_start"));
        assert!(forward.response.is_stream);

        let final_log = inner.final_response.as_ref().unwrap();
        assert!(final_log.note.contains("passthrough"));
    }

    #[test]
    fn capture_failed_forward_keeps_request_without_response() {
        let capture = ApiLogCapture::new("req-fail".into(), "codex".into(), "gpt-5.6".into());
        capture.record_forward_request(
            "provider-1",
            "P1",
            "https://gw.example.com/v1/responses",
            "POST",
            &http::HeaderMap::new(),
            b"{}",
        );
        // 发送失败：没有 record_forward_response_head，response 保持空壳
        let inner = capture.inner.lock().unwrap();
        assert_eq!(inner.forwards.len(), 1);
        assert_eq!(inner.forwards[0].response.status, None);
        assert!(inner.final_response.is_none());
    }

    #[test]
    fn retention_removes_oldest_files() {
        let dir = std::env::temp_dir().join(format!(
            "cc-switch-api-log-test-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..5 {
            std::fs::write(dir.join(format!("20260101_00000{i}.json")), "{}").unwrap();
        }
        // 一个非 json 文件不应参与清理
        std::fs::write(dir.join("notes.txt"), "").unwrap();

        enforce_retention(&dir, 3).unwrap();
        let mut remaining: Vec<String> = std::fs::read_dir(&dir)
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().to_string())
            .collect();
        remaining.sort();
        assert_eq!(
            remaining,
            vec![
                "20260101_000002.json".to_string(),
                "20260101_000003.json".to_string(),
                "20260101_000004.json".to_string(),
                "notes.txt".to_string(),
            ]
        );
        std::fs::remove_dir_all(&dir).ok();
    }
}
