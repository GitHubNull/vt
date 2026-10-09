//! AppError 统一错误类型（tech/04 4.6）。
//!
//! 错误编号前缀约定（编号出现在用户可见提示中，编号→文案映射在 src/i18n/）：
//! - `E-AUDIO-xxx`   音频采集 / 设备错误
//! - `E-ASR-xxx`     识别引擎错误
//! - `E-INJECT-xxx`  文本上屏错误
//! - `E-CONFIG-xxx`  配置读写错误
//! - `E-NET-xxx`     网络错误
//! - `E-PERM-xxx`    权限错误
//! - `E-PLAT-xxx`    平台适配层错误

use serde::Serialize;

/// 后端统一错误：命令层返回给前端 `AppError { code, message }`（tech/03 3.5）。
// 允许 dead_code：变体为骨架，随各服务任务接入后自然被构造。
#[allow(dead_code)]
#[derive(Debug, thiserror::Error)]
pub enum AppError {
    #[error("[E-AUDIO-{0:03}] {1}")]
    Audio(u32, String),
    #[error("[E-ASR-{0:03}] {1}")]
    Asr(u32, String),
    #[error("[E-INJECT-{0:03}] {1}")]
    Inject(u32, String),
    #[error("[E-CONFIG-{0:03}] {1}")]
    Config(u32, String),
    #[error("[E-NET-{0:03}] {1}")]
    Network(u32, String),
    #[error("[E-PERM-{0:03}] {1}")]
    Permission(u32, String),
    #[error("[E-PLAT-{0:03}] {1}")]
    Platform(u32, String),
}

impl AppError {
    /// 稳定错误编号，如 `E-AUDIO-001`。
    pub fn code(&self) -> String {
        match self {
            Self::Audio(n, _) => format!("E-AUDIO-{n:03}"),
            Self::Asr(n, _) => format!("E-ASR-{n:03}"),
            Self::Inject(n, _) => format!("E-INJECT-{n:03}"),
            Self::Config(n, _) => format!("E-CONFIG-{n:03}"),
            Self::Network(n, _) => format!("E-NET-{n:03}"),
            Self::Permission(n, _) => format!("E-PERM-{n:03}"),
            Self::Platform(n, _) => format!("E-PLAT-{n:03}"),
        }
    }

    /// 人类可读的错误描述（不含编号）。
    pub fn message(&self) -> &str {
        match self {
            Self::Audio(_, m)
            | Self::Asr(_, m)
            | Self::Inject(_, m)
            | Self::Config(_, m)
            | Self::Network(_, m)
            | Self::Permission(_, m)
            | Self::Platform(_, m) => m,
        }
    }
}

impl Serialize for AppError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        let mut state = serializer.serialize_struct("AppError", 2)?;
        state.serialize_field("code", &self.code())?;
        state.serialize_field("message", self.message())?;
        state.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_code_format_is_stable() {
        assert_eq!(
            AppError::Audio(1, "mic unavailable".into()).code(),
            "E-AUDIO-001"
        );
        assert_eq!(
            AppError::Asr(42, "model missing".into()).code(),
            "E-ASR-042"
        );
        assert_eq!(
            AppError::Config(7, "bad json".into()).code(),
            "E-CONFIG-007"
        );
    }

    #[test]
    fn display_includes_code_and_message() {
        let err = AppError::Permission(3, "mic denied".into());
        assert_eq!(err.to_string(), "[E-PERM-003] mic denied");
        assert_eq!(err.message(), "mic denied");
    }

    #[test]
    fn serializes_to_code_and_message() {
        let err = AppError::Network(5, "timeout".into());
        let value = serde_json::to_value(&err);
        assert!(value.is_ok());
    }
}
