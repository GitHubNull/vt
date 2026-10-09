//! LogService 骨架：日志滚动、脱敏、导出（tech/01 1.4、tech/04 4.7、tech/08 8.4）。
//!
//! T-001 仅提供按天滚动文件追加器的创建入口；全局脱敏 Layer 与导出在后续任务实现。

use std::path::Path;

/// 日志文件名前缀。
const LOG_FILE_PREFIX: &str = "vibetype.log";

/// 日志服务（骨架）。
pub struct LogService;

impl LogService {
    /// 创建按天滚动的非阻塞文件写入器。
    ///
    /// 返回 `(writer, guard)`：`guard` 必须存活至进程退出，否则缓冲日志会丢失。
    pub fn daily_writer(
        log_dir: &Path,
    ) -> (
        tracing_appender::non_blocking::NonBlocking,
        tracing_appender::non_blocking::WorkerGuard,
    ) {
        let appender = tracing_appender::rolling::daily(log_dir, LOG_FILE_PREFIX);
        tracing_appender::non_blocking(appender)
    }
}
