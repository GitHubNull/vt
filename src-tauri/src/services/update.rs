//! UpdateService（F-ABOUT-01、tech/08 8.8）：版本检查客户端逻辑。
//!
//! - NFR-PR-06：请求只发送版本号 / OS / 架构（URL query），不带任何用户数据；
//! - v0.1 更新服务器未就绪：客户端逻辑完整实现（拉取可配置地址 → 解析 → 比较），
//!   默认地址为占位地址（example.com），网络失败返回 `unavailable` + 错误编号，
//!   不视为异常；
//! - 自动安装更新（apply_update）v0.1 不提供：Tauri Updater 与签名公钥配置属
//!   发布流程事项，调用返回明确错误（E-NET-002），不静默忽略。

use serde::Deserialize;

/// 检查更新错误编号：网络不可达 / 服务器未就绪。
pub const ERR_CHECK_UNAVAILABLE: u32 = 1;
/// 检查更新错误编号：v0.1 自动安装更新未提供。
pub const ERR_APPLY_UNSUPPORTED: u32 = 2;

/// 更新检查结果（与前端 `UpdateCheckResult` 逐字段对应，T-016 契约）。
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
pub struct UpdateCheckResult {
    pub status: UpdateStatus,
    pub version: Option<String>,
    pub notes: Option<String>,
    pub error_code: Option<String>,
}

/// 更新状态。
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum UpdateStatus {
    /// 已是最新。
    UpToDate,
    /// 有新版本可用。
    Available,
    /// 无法检查（网络失败 / 服务器未就绪）。
    Unavailable,
}

/// 更新源响应（可配置地址的契约格式，tech/08 8.8）。
#[derive(Debug, Deserialize)]
struct FeedRelease {
    version: String,
    #[serde(default)]
    notes: Option<String>,
}

/// UpdateService：无状态客户端（每次检查新建请求，5 s 超时）。
pub struct UpdateService {
    client: reqwest::Client,
}

impl UpdateService {
    pub fn new() -> Self {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .expect("reqwest 客户端构造失败");
        Self { client }
    }

    /// 检查更新：仅发送版本号 / OS / 架构（NFR-PR-06）。
    pub async fn check_update(&self, feed_url: &str, current_version: &str) -> UpdateCheckResult {
        // 请求载荷：版本号 / OS / 架构，query 参数显式列出，无其他数据。
        let request = self
            .client
            .get(feed_url)
            .query(&[
                ("current_version", current_version),
                ("os", std::env::consts::OS),
                ("arch", std::env::consts::ARCH),
            ])
            .send()
            .await;

        let response = match request {
            Ok(response) => response,
            Err(err) => {
                tracing::warn!(error = %err, "检查更新失败（网络不可达或服务器未就绪）");
                return UpdateCheckResult::unavailable(ERR_CHECK_UNAVAILABLE);
            }
        };
        if !response.status().is_success() {
            tracing::warn!(status = %response.status(), "更新源返回非成功状态");
            return UpdateCheckResult::unavailable(ERR_CHECK_UNAVAILABLE);
        }
        let release: FeedRelease = match response.json().await {
            Ok(release) => release,
            Err(err) => {
                tracing::warn!(error = %err, "更新源响应解析失败");
                return UpdateCheckResult::unavailable(ERR_CHECK_UNAVAILABLE);
            }
        };
        if version_newer(&release.version, current_version) {
            UpdateCheckResult {
                status: UpdateStatus::Available,
                version: Some(release.version),
                notes: release.notes,
                error_code: None,
            }
        } else {
            UpdateCheckResult {
                status: UpdateStatus::UpToDate,
                version: None,
                notes: None,
                error_code: None,
            }
        }
    }
}

impl Default for UpdateService {
    fn default() -> Self {
        Self::new()
    }
}

impl UpdateCheckResult {
    fn unavailable(code: u32) -> Self {
        Self {
            status: UpdateStatus::Unavailable,
            version: None,
            notes: None,
            error_code: Some(format!("E-NET-{code:03}")),
        }
    }
}

/// `a` 是否比 `b` 新：按 `.` 分隔的数字段比较（忽略前导 v，段不足补 0）。
pub fn version_newer(a: &str, b: &str) -> bool {
    fn segments(version: &str) -> Vec<u64> {
        version
            .trim_start_matches('v')
            .split('.')
            .map(|segment| segment.parse::<u64>().unwrap_or(0))
            .collect()
    }
    let (a_segments, mut b_segments) = (segments(a), segments(b));
    b_segments.resize(a_segments.len(), 0);
    let mut b_iter = b_segments.iter();
    for a_segment in &a_segments {
        let b_segment = b_iter.next().copied().unwrap_or(0);
        if *a_segment != b_segment {
            return *a_segment > b_segment;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_semantics() {
        assert!(version_newer("0.2.0", "0.1.0"));
        assert!(version_newer("0.1.1", "0.1.0"));
        assert!(!version_newer("0.1.0", "0.1.0"));
        assert!(!version_newer("0.1.0", "0.2.0"));
        assert!(version_newer("v1.0.0", "0.9.9"));
        // 段数不齐补 0：1.0 == 1.0.0。
        assert!(!version_newer("1.0", "1.0.0"));
        assert!(version_newer("1.0.1", "1.0"));
    }
}
