//! Windows 原生 OCR —— 通过 Windows.Media.Ocr (Win10 1809+) 识别图片文字。
//!
//! 加载图片字节 → InMemoryRandomAccessStream → BitmapDecoder → SoftwareBitmap
//! → OcrEngine → RecognizeAsync → 拼接行文本。

#![cfg(target_os = "windows")]

use windows::core::HSTRING;
use windows::Graphics::Imaging::BitmapDecoder;
use windows::Media::Ocr::{IOcrEngineStatics, OcrEngine};
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};

/// 对指定图片文件路径执行 OCR，返回识别文本（按行拼接）。
pub fn recognize_file(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取截图失败: {e}"))?;
    recognize_bytes(&bytes)
}

/// 对内存中的图片字节执行 OCR。
pub fn recognize_bytes(bytes: &[u8]) -> Result<String, String> {
    let stream = InMemoryRandomAccessStream::new()
        .map_err(|e| format!("创建内存流失败: {e}"))?;

    // 通过 DataWriter 写入图片字节
    {
        let writer = DataWriter::new().map_err(|e| format!("创建 DataWriter 失败: {e}"))?;
        writer
            .WriteBytes(bytes)
            .map_err(|e| format!("写入字节失败: {e}"))?;
        writer
            .StoreAsync()
            .map_err(|e| format!("存储数据失败: {e}"))?
            .get()
            .map_err(|e| format!("等待存储完成失败: {e}"))?;
        writer
            .DetachStream()
            .map_err(|e| format!("分离流失败: {e}"))?;
    }

    // 流位置已在写入后推进，Seek 回开头供解码器读取
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

    // 创建 OCR 引擎 —— 优先中文简体，回退到用户配置语言
    let engine = create_ocr_engine().ok_or("无法创建 OCR 引擎，请确认 Windows 语言包已安装")?;

    // 执行 OCR
    let result = engine
        .RecognizeAsync(&bitmap)
        .map_err(|e| format!("OCR 识别失败: {e}"))?
        .get()
        .map_err(|e| format!("等待 OCR 完成失败: {e}"))?;

    // 拼接识别文本 —— OcrResult.Text() 返回完整文本
    let text = result
        .Text()
        .map_err(|e| format!("获取识别文本失败: {e}"))?;

    Ok(text.to_string())
}

/// 创建 OCR 引擎：尝试中文简体 → 用户配置语言 → 系统默认。
fn create_ocr_engine() -> Option<OcrEngine> {
    // 通过 IOcrEngineStatics 接口访问 TryCreateFromLanguage
    let result = OcrEngine::IOcrEngineStatics(|statics| {
        // 优先尝试中文简体
        let zh_lang = HSTRING::from("zh-Hans");
        if let Ok(engine) = statics.TryCreateFromLanguage(&zh_lang) {
            return Ok(engine);
        }
        // 回退到用户配置的语言
        if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
            return Ok(engine);
        }
        Err(windows::core::Error::from_win32())
    });

    result.ok()
}
