//! 系统托盘：图标、最小菜单，以及关窗驻留相关的共用入口。

use tauri::menu::{MenuBuilder, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};

/// 托盘图标 id；退出前通过 `AppHandle::tray_by_id` 取回句柄显式摘除。
pub const TRAY_ID: &str = "agent-manager-tray";

const MENU_SHOW_MAIN: &str = "show_main";
const MENU_SEARCH: &str = "open_search";
const MENU_TAKEOVER: &str = "open_takeover";
const MENU_QUIT: &str = "quit";

/// 托盘发给界面的动作事件名；载荷是界面侧认识的动作 id。
pub const TRAY_ACTION_EVENT: &str = "tray-action";

/// 恢复并聚焦主窗口；托盘「显示主界面」、单实例二次启动回调与
/// macOS Dock Reopen 共用同一条路径。
pub fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
}

/// 托盘只负责「把人带到能做这件事的地方」，不直接触发写操作：
/// 真正的执行仍要在界面上点，锁、确认与审计因此都走同一条既有路径。
fn handle_menu_event(app: &AppHandle, event_id: &str) {
    match event_id {
        MENU_SHOW_MAIN => show_main_window(app),
        MENU_SEARCH => {
            show_main_window(app);
            let _ = app.emit(TRAY_ACTION_EVENT, "search");
        }
        MENU_TAKEOVER => {
            show_main_window(app);
            let _ = app.emit(TRAY_ACTION_EVENT, "takeover");
        }
        // 退出统一走 app.exit(0) → RunEvent::ExitRequested，由 main.rs 收尾摘托盘图标。
        MENU_QUIT => app.exit(0),
        _ => {}
    }
}

/// icons/32x32.png 是彩色应用图标而非 macOS 单色模板图，不设 `icon_as_template`
///（否则状态栏会渲染成纯黑剪影）；彩色图标在深浅色菜单栏下均可读。
fn tray_icon(app: &tauri::App) -> Option<tauri::image::Image<'static>> {
    const ICON_BYTES: &[u8] = include_bytes!("../icons/32x32.png");
    tauri::image::Image::from_bytes(ICON_BYTES)
        .ok()
        .or_else(|| {
            // default_window_icon 的借用绑在 &App 上，拷贝成 owned 再交给托盘。
            let icon = app.default_window_icon()?;
            Some(tauri::image::Image::new_owned(
                icon.rgba().to_vec(),
                icon.width(),
                icon.height(),
            ))
        })
}

pub fn setup(app: &tauri::App) -> tauri::Result<()> {
    let show_main = MenuItem::with_id(app, MENU_SHOW_MAIN, "显示主界面", true, None::<&str>)?;
    let search = MenuItem::with_id(app, MENU_SEARCH, "搜索与执行…", true, None::<&str>)?;
    let takeover = MenuItem::with_id(app, MENU_TAKEOVER, "接管中心", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "退出", true, None::<&str>)?;
    let menu = MenuBuilder::new(app)
        .item(&show_main)
        .separator()
        .item(&search)
        .item(&takeover)
        .separator()
        .item(&quit)
        .build()?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("Agent Manager")
        .menu(&menu)
        .on_menu_event(|app, event| handle_menu_event(app, event.id.as_ref()))
        .show_menu_on_left_click(true);
    // 内嵌 PNG 解码失败时退到打包默认窗口图标；两者皆无只打诊断、不阻断启动。
    match tray_icon(app) {
        Some(icon) => builder = builder.icon(icon),
        None => eprintln!("tray icon unavailable; building tray without icon"),
    }
    builder.build(app)?;
    Ok(())
}

/// 退出前显式摘除托盘图标。进程若绕过 `TrayIcon::drop`（如 std::process::exit），
/// Windows Shell 收不到 NIM_DELETE，会残留一个悬停才消失的死图标；
/// `set_visible(false)` 在各平台都是正常的隐藏/移除语义，作为统一收尾。
pub fn remove_tray_icon_before_exit(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        let _ = tray.set_visible(false);
    }
}
