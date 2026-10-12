//! OCR 模块入口。

#[cfg(target_os = "macos")]
mod image;

#[cfg(target_os = "macos")]
pub mod vision;

#[cfg(all(test, target_os = "macos"))]
mod tests;

/// 对图片文件执行 OCR，返回识别文本。
#[cfg(target_os = "macos")]
pub fn recognize_file(path: &str) -> Result<String, String> {
    vision::recognize_file(path)
}

#[cfg(not(target_os = "macos"))]
pub fn recognize_file(_path: &str) -> Result<String, String> {
    Err("当前平台暂不支持 OCR".to_string())
}
