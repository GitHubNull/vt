//! Linux 前台窗口识别（T-501，tech/09 9.2.3）。
//!
//! - **X11**：读根窗口 `_NET_ACTIVE_WINDOW`，再取该窗口的 WM_CLASS（应用标识）、
//!   `_NET_WM_PID`（→ `/proc/<pid>/comm` 进程名）与 `_NET_WM_NAME` / WM_NAME（标题）；
//! - **Wayland**：协议限制无全局活动窗口查询，`x11rb::connect` 失败 → 返回空
//!   [`AppInfo`]，服务层映射为 Unknown 场景（SPEC T-501 手动切换兜底；
//!   完整适配见 v1.0 T-1038）。XWayland 会话下连接成功，行为与 X11 一致（尽力而为）；
//! - 任何一步读取失败都静默降级为空 [`AppInfo`]，不 panic、不写包含识别信息的日志。
//!
//! 最小权限说明（AGENTS.md 3.5）：仅连接本机 X display **只读**窗口元数据
//! （`_NET_ACTIVE_WINDOW` / WM_CLASS / `_NET_WM_NAME` / `_NET_WM_PID` property），
//! 只读 `/proc/<pid>/comm`；不注入事件、不监听其他窗口、不修改任何窗口属性；
//! 读取到的窗口标题等识别信息不写日志（NFR-S-02）。

use x11rb::atom_manager;
use x11rb::connection::Connection;
use x11rb::protocol::xproto::{AtomEnum, ConnectionExt, GetPropertyReply};
use x11rb::rust_connection::RustConnection;

use super::AppInfo;

/// 前台窗口信息提供者（X11 实现；Wayland 下静默返回空 `AppInfo`）。
pub struct X11WindowInfo;

impl super::WindowInfoProvider for X11WindowInfo {
    fn active_app(&self) -> AppInfo {
        active_app()
    }
}

// 本模块用到的 X atoms（`atom_manager!` 一次往返批量 intern）。
atom_manager! {
    Atoms: AtomsCookie {
        _NET_ACTIVE_WINDOW,
        _NET_WM_PID,
        _NET_WM_NAME,
        WM_CLASS,
        UTF8_STRING,
    }
}

/// 读取前台应用信息；任何失败路径返回空 [`AppInfo`]（Unknown 场景）。
fn active_app() -> AppInfo {
    read_active_app().unwrap_or_default()
}

/// 全流程：连接 → 活动窗口 → WM_CLASS / 进程名 / 标题。
fn read_active_app() -> Option<AppInfo> {
    let (conn, screen) = x11rb::connect(None).ok()?;
    let root = conn.setup().roots.get(screen)?.root;
    let atoms = Atoms::new(&conn).ok()?.reply().ok()?;

    // _NET_ACTIVE_WINDOW：EWMH 规范的当前活动窗口（0 = 无前台窗口）。
    let window = prop_u32(&conn, root, atoms._NET_ACTIVE_WINDOW, AtomEnum::WINDOW)?
        .first()
        .copied()
        .filter(|window| *window != x11rb::NONE)?;

    let wm_class = read_wm_class(&conn, window, atoms.WM_CLASS);
    // 应用显示名以 WM_CLASS class 段为准（X11 无统一的应用显示名来源；
    // 如 VS Code 为 "Code"，GNOME Terminal 为 "gnome-terminal-server"）。
    let name = wm_class.clone().unwrap_or_default();
    // 进程名优先 /proc/<pid>/comm（EWMH 进程可执行名，如 "code"），回退 WM_CLASS。
    let process = wm_pid(&conn, window, atoms._NET_WM_PID)
        .and_then(proc_comm)
        .or(wm_class)
        .unwrap_or_default();

    Some(AppInfo {
        name,
        process,
        title: read_title(&conn, window, atoms._NET_WM_NAME),
    })
}

/// 读取 32 位整型 property 的首个值（如 `_NET_ACTIVE_WINDOW` / `_NET_WM_PID`）。
fn prop_u32(
    conn: &RustConnection,
    window: x11rb::protocol::xproto::Window,
    property: u32,
    ty: AtomEnum,
) -> Option<Vec<u32>> {
    conn.get_property(false, window, property, ty, 0, 1)
        .ok()?
        .reply()
        .ok()?
        .value32()
        .map(Iterator::collect)
}

/// 读取 WM_CLASS 的 class 段（格式：`instance\0class\0`，LATIN-1/UTF-8 混用历史原因，
/// class 段按 ASCII 处理；规则匹配层大小写不敏感）。
fn read_wm_class(
    conn: &RustConnection,
    window: x11rb::protocol::xproto::Window,
    atom: u32,
) -> Option<String> {
    let reply = prop_bytes(conn, window, atom, AtomEnum::STRING)?;
    // 第一段 instance，第二个 \0 前为 class 段。
    let bytes = &reply.value;
    let class_start = bytes.iter().position(|&b| b == 0)? + 1;
    let rest = &bytes[class_start..];
    let class_end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    let class = String::from_utf8_lossy(&rest[..class_end])
        .trim()
        .to_string();
    if class.is_empty() {
        None
    } else {
        Some(class)
    }
}

/// 读取任意 8-bit property 原始字节。
fn prop_bytes(
    conn: &RustConnection,
    window: x11rb::protocol::xproto::Window,
    property: u32,
    ty: AtomEnum,
) -> Option<GetPropertyReply> {
    conn.get_property(false, window, property, ty, 0, 8192)
        .ok()?
        .reply()
        .ok()
}

/// 读取 `_NET_WM_PID`。
fn wm_pid(
    conn: &RustConnection,
    window: x11rb::protocol::xproto::Window,
    atom: u32,
) -> Option<u32> {
    prop_u32(conn, window, atom, AtomEnum::CARDINAL)?
        .first()
        .copied()
}

/// 读取进程可执行名（`/proc/<pid>/comm`，无路径穿越风险：pid 来自内核 property）。
fn proc_comm(pid: u32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    let comm = comm.trim().to_string();
    if comm.is_empty() {
        None
    } else {
        Some(comm)
    }
}

/// 读取窗口标题：`_NET_WM_NAME`（UTF-8，EWMH）优先，回退 `WM_NAME`（可能为
/// COMPOUND_TEXT，lossy 转换）；失败返回空串。标题不写日志（NFR-S-02）。
fn read_title(
    conn: &RustConnection,
    window: x11rb::protocol::xproto::Window,
    net_wm_name: u32,
) -> String {
    let net_name = prop_bytes(conn, window, net_wm_name, AtomEnum::STRING)
        .map(|reply| String::from_utf8_lossy(&reply.value).trim().to_string());
    match net_name {
        Some(name) if !name.is_empty() => return name,
        _ => {}
    }
    prop_bytes(conn, window, AtomEnum::WM_NAME.into(), AtomEnum::STRING)
        .map(|reply| String::from_utf8_lossy(&reply.value).trim().to_string())
        .unwrap_or_default()
}
