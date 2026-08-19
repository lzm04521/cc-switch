//! 托盘展示用的用量缓存（进程内、写穿式）。
//!
//! 各 usage 查询命令成功时写入；系统托盘构建菜单时读取。不持久化，
//! 进程重启即空，由下一次自动查询或托盘悬停触发的刷新重新填充。

use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::Serialize;

use crate::app_config::AppType;
use crate::provider::UsageResult;
use crate::services::subscription::SubscriptionQuota;

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// 脚本型用量缓存条目：结果 + 查询时刻（悬浮球面板显示"x 分钟前"用）
#[derive(Debug, Clone)]
struct ScriptCacheEntry {
    result: UsageResult,
    queried_at: i64,
}

/// 全量快照条目（悬浮球面板按 app/provider 匹配消费）
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UsageScriptSnapshot {
    pub app_type: String,
    pub provider_id: String,
    pub result: UsageResult,
    pub queried_at: i64,
}

#[derive(Default)]
pub struct UsageCache {
    subscription: RwLock<HashMap<AppType, SubscriptionQuota>>,
    script: RwLock<HashMap<(AppType, String), ScriptCacheEntry>>,
}

impl UsageCache {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn put_subscription(&self, app_type: AppType, quota: SubscriptionQuota) {
        if let Ok(mut w) = self.subscription.write() {
            w.insert(app_type, quota);
        }
    }

    pub fn put_script(&self, app_type: AppType, provider_id: String, result: UsageResult) {
        if let Ok(mut w) = self.script.write() {
            w.insert(
                (app_type, provider_id),
                ScriptCacheEntry {
                    result,
                    queried_at: now_millis(),
                },
            );
        }
    }

    /// 以借用形式暴露订阅快照，避免托盘每次重建时深拷贝整个 `SubscriptionQuota`。
    pub fn with_subscription<R>(
        &self,
        app_type: &AppType,
        f: impl FnOnce(&SubscriptionQuota) -> R,
    ) -> Option<R> {
        self.subscription
            .read()
            .ok()
            .and_then(|r| r.get(app_type).map(f))
    }

    /// 以借用形式暴露脚本型用量结果，同上。
    pub fn with_script<R>(
        &self,
        app_type: &AppType,
        provider_id: &str,
        f: impl FnOnce(&UsageResult) -> R,
    ) -> Option<R> {
        self.script
            .read()
            .ok()
            .and_then(|r| {
                r.get(&(app_type.clone(), provider_id.to_string()))
                    .map(|entry| f(&entry.result))
            })
    }

    /// 全量脚本缓存快照（悬浮球面板一次取走所有 provider 的结果与查询时刻）。
    pub fn snapshot_scripts(&self) -> Vec<UsageScriptSnapshot> {
        self.script
            .read()
            .map(|r| {
                r.iter()
                    .map(|((app_type, provider_id), entry)| UsageScriptSnapshot {
                        app_type: app_type.as_str().to_string(),
                        provider_id: provider_id.clone(),
                        result: entry.result.clone(),
                        queried_at: entry.queried_at,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn invalidate_script(&self, app_type: &AppType, provider_id: &str) {
        // 热路径会对每个禁用脚本的 provider 在托盘重建时调用一次：先走读锁
        // `contains_key` 快速放行"本来就不在缓存里"的常见情况，避免无谓的写锁升级。
        let key = (app_type.clone(), provider_id.to_string());
        if !self.script.read().is_ok_and(|r| r.contains_key(&key)) {
            return;
        }
        if let Ok(mut w) = self.script.write() {
            w.remove(&key);
        }
    }

    pub fn invalidate_subscription(&self, app_type: &AppType) {
        if !self
            .subscription
            .read()
            .is_ok_and(|r| r.contains_key(app_type))
        {
            return;
        }
        if let Ok(mut w) = self.subscription.write() {
            w.remove(app_type);
        }
    }

    /// Drop all process-local usage snapshots after a database/provider restore.
    pub fn invalidate_all(&self) {
        if let Ok(mut subscriptions) = self.subscription.write() {
            subscriptions.clear();
        }
        if let Ok(mut scripts) = self.script.write() {
            scripts.clear();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::services::subscription::CredentialStatus;

    fn fake_quota() -> SubscriptionQuota {
        SubscriptionQuota {
            tool: "claude".to_string(),
            credential_status: CredentialStatus::Valid,
            credential_message: None,
            success: true,
            tiers: vec![],
            extra_usage: None,
            error: None,
            queried_at: Some(0),
        }
    }

    fn fake_result() -> UsageResult {
        UsageResult {
            success: true,
            data: None,
            error: None,
        }
    }

    #[test]
    fn subscription_round_trip() {
        let cache = UsageCache::new();
        assert!(cache
            .with_subscription(&AppType::Claude, |q| q.success)
            .is_none());
        cache.put_subscription(AppType::Claude, fake_quota());
        let got = cache
            .with_subscription(&AppType::Claude, |q| q.success)
            .unwrap();
        assert!(got);
        assert!(cache
            .with_subscription(&AppType::Codex, |q| q.success)
            .is_none());
    }

    #[test]
    fn script_round_trip_and_invalidate() {
        let cache = UsageCache::new();
        assert!(cache
            .with_script(&AppType::Codex, "pid", |r| r.success)
            .is_none());
        cache.put_script(AppType::Codex, "pid".to_string(), fake_result());
        assert!(cache
            .with_script(&AppType::Codex, "pid", |r| r.success)
            .is_some());
        cache.invalidate_script(&AppType::Codex, "pid");
        assert!(cache
            .with_script(&AppType::Codex, "pid", |r| r.success)
            .is_none());
    }

    #[test]
    fn script_keys_isolated_by_app_type() {
        let cache = UsageCache::new();
        cache.put_script(AppType::Claude, "same".to_string(), fake_result());
        assert!(cache
            .with_script(&AppType::Claude, "same", |r| r.success)
            .is_some());
        assert!(cache
            .with_script(&AppType::Codex, "same", |r| r.success)
            .is_none());
    }

    #[test]
    fn invalidate_all_clears_subscription_and_script_snapshots() {
        let cache = UsageCache::new();
        cache.put_subscription(AppType::Claude, fake_quota());
        cache.put_script(AppType::Codex, "provider".to_string(), fake_result());

        cache.invalidate_all();

        assert!(cache
            .with_subscription(&AppType::Claude, |quota| quota.success)
            .is_none());
        assert!(cache
            .with_script(&AppType::Codex, "provider", |usage| usage.success)
            .is_none());
}

    fn snapshot_scripts_covers_all_apps_with_queried_at() {
        let before = now_millis();
        let cache = UsageCache::new();
        cache.put_script(AppType::Claude, "a".to_string(), fake_result());
        cache.put_script(
            AppType::Codex,
            "b".to_string(),
            UsageResult {
                success: false,
                data: None,
                error: Some("boom".to_string()),
            },
        );
        // 失败结果同样进缓存（悬浮球面板要显示失败态，不吞错）
        cache.invalidate_script(&AppType::Codex, "missing");

        let mut snap = cache.snapshot_scripts();
        snap.sort_by(|x, y| x.app_type.cmp(&y.app_type));
        assert_eq!(snap.len(), 2);
        assert_eq!(snap[0].app_type, "claude");
        assert_eq!(snap[0].provider_id, "a");
        assert!(snap[0].queried_at >= before);
        assert_eq!(snap[1].app_type, "codex");
        assert!(!snap[1].result.success);
        assert_eq!(snap[1].result.error.as_deref(), Some("boom"));
    }
}
