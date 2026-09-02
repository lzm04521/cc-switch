//! Provider 用量后台周期刷新（fork 增强：「自动刷新所有 Provider 用量」）。
//!
//! 开关开启时，用量定时查询不再依赖主窗口前端：`UsageFooter` 的
//! react-query 轮询挂在组件 observer 上，主窗口隐藏到托盘（WebView
//! 节流）或切到非 providers 视图（组件卸载）后轮询停止，`UsageCache`
//! 不再被写入，悬浮球面板（只读镜像）随之停更。本任务在 Rust 侧按
//! 各 Provider 的 `autoQueryInterval` 周期调 `queryProviderUsage`
//! （复用其 put_script + emit 写穿逻辑），查询源脱离 WebView 生命周期。

use std::collections::HashMap;
use std::time::Duration;

use futures::future::join_all;
use tauri::Manager;

use crate::app_config::AppType;
use crate::store::AppState;

/// 任务轮询粒度。间隔下限 5 分钟，60s 检查一次，最坏延迟一轮 tick。
const TICK_SECS: u64 = 60;
/// 后台刷新间隔下限（分钟），与前端 BACKGROUND_AUTO_QUERY_MIN_MINUTES 一致。
const BACKGROUND_REFRESH_MIN_MINUTES: u64 = 5;

/// 到期判定：距「任务上次尝试（含失败）与缓存上次成功」中较晚者
/// ≥ 生效间隔（`max(interval, 5)` 分钟）才需要查询。
///
/// - 尝试时刻含失败：第三方端点故障时不会每 tick 重试形成请求风暴；
/// - 缓存成功时刻纳入比较：主窗口前端轮询刚查过的 Provider 自动跳过，
///   前后端两条查询路径不会对同一 Provider 双发。
pub(crate) fn is_due(
    interval_minutes: u64,
    last_attempt_ms: Option<i64>,
    cached_queried_at_ms: Option<i64>,
    now_ms: i64,
) -> bool {
    let effective_ms = interval_minutes.max(BACKGROUND_REFRESH_MIN_MINUTES) * 60 * 1000;
    match last_attempt_ms.max(cached_queried_at_ms) {
        Some(last) => now_ms.saturating_sub(last) >= effective_ms as i64,
        None => true, // 从未查过：立即纳入首轮
    }
}

/// 启动后台用量刷新循环（app setup 调一次；每轮热读设置开关，改设置即时生效）。
pub(crate) fn spawn_usage_refresher(app: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(TICK_SECS));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        interval.tick().await; // 跳过立即触发的首个 tick，避开启动期首轮查询
        // 任务维度的上次尝试时刻（epoch ms），成功失败都记
        let mut last_attempts: HashMap<(String, String), i64> = HashMap::new();
        loop {
            interval.tick().await;
            run_refresh_cycle(&app, &mut last_attempts).await;
        }
    });
}

async fn run_refresh_cycle(
    app: &tauri::AppHandle,
    last_attempts: &mut HashMap<(String, String), i64>,
) {
    // 开关关：维持现状（当前 Provider 由主窗口前端轮询与托盘悬停覆盖）
    if !crate::settings::get_settings().auto_refresh_all_providers_usage {
        return;
    }

    let app_state = app.state::<AppState>();
    let visible_apps = crate::settings::get_settings()
        .visible_apps
        .unwrap_or_default();
    let now_ms = crate::services::usage_cache::now_millis();

    let mut due = Vec::new();
    for app_type in AppType::all() {
        // zcode / dsh 的 provider 由应用内自管，cc-switch 不查询（与悬浮球/托盘口径一致）
        if matches!(app_type, AppType::Zcode | AppType::Dsh) {
            continue;
        }
        if !visible_apps.is_visible(&app_type) {
            continue;
        }
        let providers = match app_state.db.get_all_providers(app_type.as_str()) {
            Ok(p) => p,
            Err(e) => {
                log::warn!(
                    "[UsageRefresher] 读取 {} 供应商列表失败: {e}",
                    app_type.as_str()
                );
                continue;
            }
        };
        for (provider_id, provider) in providers {
            if !provider.has_usage_script_enabled() {
                continue;
            }
            // autoQueryInterval 为 0/未设置 = 关闭该 Provider 的自动查询（与前端 UsageFooter 口径一致）
            let Some(interval_min) = provider
                .meta
                .as_ref()
                .and_then(|m| m.usage_script.as_ref())
                .and_then(|s| s.auto_query_interval)
                .filter(|n| *n > 0)
            else {
                continue;
            };
            let key = (app_type.as_str().to_string(), provider_id.clone());
            let cached = app_state
                .usage_cache
                .script_queried_at(&app_type, &provider_id);
            if !is_due(
                interval_min,
                last_attempts.get(&key).copied(),
                cached,
                now_ms,
            ) {
                continue;
            }
            last_attempts.insert(key, now_ms);
            due.push((app_type.clone(), provider_id));
        }
    }

    if due.is_empty() {
        return;
    }
    log::debug!(
        "[UsageRefresher] 本轮到期刷新 {} 个供应商用量",
        due.len()
    );

    let mut futures = Vec::new();
    for (app_type, provider_id) in due {
        let app_clone = app.clone();
        let state = app.state::<AppState>();
        let copilot_state = app.state::<crate::commands::CopilotAuthState>();
        let xai_state = app.state::<crate::commands::XaiOAuthState>();
        let app_str = app_type.as_str().to_string();
        futures.push(async move {
            // 写穿缓存（put_script + emit usage-cache-updated）由命令内部完成；
            // 确定性失败（Ok success:false）同样写缓存并广播，瞬时失败（Err）不写
            if let Err(e) = crate::commands::queryProviderUsage(
                app_clone,
                state,
                copilot_state,
                xai_state,
                provider_id.clone(),
                app_str.clone(),
            )
            .await
            {
                log::debug!(
                    "[UsageRefresher] 刷新 {app_str} 供应商 {provider_id} 用量失败: {e}"
                );
            }
        });
    }
    join_all(futures).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIN: i64 = 60 * 1000;

    #[test]
    fn never_queried_is_due() {
        assert!(is_due(5, None, None, 0));
    }

    #[test]
    fn fresh_cache_skips() {
        // 配置 5 分钟，缓存 3 分钟前成功（< 5 分钟下限）→ 跳过
        assert!(!is_due(5, None, Some(3 * MIN), 6 * MIN));
        // 配置 10 分钟（> 下限按设置走），缓存 6 分钟前 → 跳过
        assert!(!is_due(10, None, Some(6 * MIN), 12 * MIN));
    }

    #[test]
    fn stale_cache_is_due() {
        // 配置 10 分钟，缓存 11 分钟前 → 到期
        assert!(is_due(10, None, Some(11 * MIN), 22 * MIN));
        // 配置 0（异常值）钳制为 5 分钟下限，缓存 6 分钟前 → 到期
        assert!(is_due(0, None, Some(6 * MIN), 12 * MIN));
    }

    #[test]
    fn failed_attempt_throttles_retry() {
        // 缓存 8 分钟前已过期，但 2 分钟前任务尝试失败过 → 取较晚者，未到期
        assert!(!is_due(5, Some(2 * MIN), Some(8 * MIN), 10 * MIN));
        // 距失败尝试已 5 分钟（缓存 5 分钟前成功，较晚者同样是它）→ 到期重试
        assert!(is_due(5, Some(5 * MIN), Some(8 * MIN), 13 * MIN));
    }
}
