//! 平台开机自启管理。

use auto_launch::{AutoLaunch, AutoLaunchBuilder};

const APP_NAME: &str = "TTREE";

fn build_auto_launch() -> Result<AutoLaunch, String> {
    let exe = std::env::current_exe().map_err(|e| format!("获取当前可执行文件路径失败: {e}"))?;
    // 若运行在 TTREE.app/Contents/MacOS/ttree 内，优先传入 .app bundle 路径
    let app_path = exe
        .parent()
        .and_then(|macos| macos.parent())
        .and_then(|contents| contents.parent())
        .filter(|bundle| bundle.extension().is_some_and(|ext| ext == "app"))
        .unwrap_or(exe.as_path());

    let mut builder = AutoLaunchBuilder::new();
    builder
        .set_app_name(APP_NAME)
        .set_app_path(&app_path.to_string_lossy())
        .set_use_launch_agent(true);
    #[cfg(target_os = "windows")]
    builder.set_args(&["--hidden"]);
    builder
        .build()
        .map_err(|e| format!("初始化开机自启失败: {e}"))
}

pub fn is_enabled() -> bool {
    build_auto_launch()
        .and_then(|al| al.is_enabled().map_err(|e| e.to_string()))
        .unwrap_or(false)
}

pub fn set_enabled(enable: bool) -> Result<bool, String> {
    let al = build_auto_launch()?;
    if enable {
        al.enable().map_err(|e| format!("启用开机自启失败: {e}"))?;
    } else if al.is_enabled().unwrap_or(false) {
        al.disable().map_err(|e| format!("关闭开机自启失败: {e}"))?;
    }
    al.is_enabled()
        .map_err(|e| format!("读取开机自启状态失败: {e}"))
}
