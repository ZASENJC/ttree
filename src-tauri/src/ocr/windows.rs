//! Windows OCR —— 通过 Windows.Media.Ocr (Win10 1809+) 识别图片文字。
//!
//! 使用纯 WinRT FFI 避免 `windows` crate 版本间 API 差异。
//! 若系统未安装 OCR 语言包，返回提示信息。

#![cfg(target_os = "windows")]

/// 对指定图片文件路径执行 OCR，返回识别文本。
pub fn recognize_file(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取截图失败: {e}"))?;
    recognize_bytes(&bytes)
}

/// 对内存中的图片字节执行 OCR。
///
/// 使用 Windows.Media.Ocr WinRT API：
/// 1. 将图片字节写入 InMemoryRandomAccessStream
/// 2. BitmapDecoder 解码为 SoftwareBitmap
/// 3. OcrEngine 识别文字
pub fn recognize_bytes(bytes: &[u8]) -> Result<String, String> {
    use windows::Graphics::Imaging::BitmapDecoder;
    use windows::Media::Ocr::OcrEngine;
    use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

    // 创建内存流
    let stream = InMemoryRandomAccessStream::new()
        .map_err(|e| format!("创建内存流失败: {e}"))?;

    // 获取流的输出端并写入图片字节
    let output = stream
        .GetOutputStreamAt(0)
        .map_err(|e| format!("获取输出流失败: {e}"))?;
    let writer = DataWriter::CreateDataWriter(&output)
        .map_err(|e| format!("创建 DataWriter 失败: {e}"))?;
    writer
        .WriteBytes(bytes)
        .map_err(|e| format!("写入字节失败: {e}"))?;
    writer
        .StoreAsync()
        .map_err(|e| format!("存储数据失败: {e}"))?
        .get()
        .map_err(|e| format!("等待存储完成失败: {e}"))?;

    // Seek 回开头供解码器读取
    stream
        .Seek(0)
        .map_err(|e| format!("重置流位置失败: {e}"))?;

    // 解码图片为 SoftwareBitmap
    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|e| format!("创建图片解码器失败: {e}"))?
        .get()
        .map_err(|e| format!("等待解码器创建完成失败: {e}"))?;

    let bitmap = decoder
        .GetSoftwareBitmapAsync()
        .map_err(|e| format!("获取位图失败: {e}"))?
        .get()
        .map_err(|e| format!("等待位图获取完成失败: {e}"))?;

    // 创建 OCR 引擎
    let engine = OcrEngine::TryCreateFromUserProfileLanguages().map_err(|e| {
        format!(
            "无法创建 OCR 引擎: {e}\n\
             请安装 Windows OCR 语言包：设置 → 时间和语言 → 语言和区域 → 添加语言 → 中文(简体)"
        )
    })?;

    // 执行 OCR
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| format!("OCR 识别失败: {e}"))?
        .get()
        .map_err(|e| format!("等待 OCR 完成失败: {e}"))?;

    let text = result
        .Text()
        .map_err(|e| format!("获取识别文本失败: {e}"))?;

    Ok(text.to_string())
}
