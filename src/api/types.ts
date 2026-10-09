/**
 * 与 Rust 侧 serde 类型对应的 TS 类型（tech/03 3.5），手工保持同步。
 */

/** 后端统一错误（AppError）序列化形态，错误编号见 src-tauri/src/error.rs。 */
export interface AppErrorShape {
  code: string
  message: string
}
