//! Windows 增强型多语言、双通道自适应 `Windows.Media.Ocr` 引擎。
//!
//! 针对旧版 Windows OCR 识别效果差的 5 项根本修复：
//! 1. **内存直构 `SoftwareBitmap`**：交互式截图直接由 `CryptographicBuffer::CreateFromByteArray` +
//!    `SoftwareBitmap::CreateCopyWithAlphaFromBuffer(Bgra8)` 构建，彻底绕过易出错的临时文件与 `BitmapDecoder`；
//! 2. **双语优先的多引擎调度**：Windows 的 `zh-Hans`（简体中文）OCR 模型天然具备「中英双语混合识别」能力，
//!    而纯 `en-US` 引擎遇到汉字会输出乱码符号。因此优先探测并启用 `zh-Hans-CN` / `zh-Hans` / `zh-Hant`，
//!    当首选引擎得分过低时自动在已安装的其余语言引擎间回退择优；
//! 3. **双通道图像预处理重试（Pass-1 自然放大延展 + Pass-2 深色反相高对比度拉伸）**：
//!    解决小字号单行截图与深色模式（白字黑底/灰字暗底）漏识问题；
//! 4. **分词级 CJK 伪空格消除**：遍历 `OcrLine::Words()`，通过 `postprocess::join_ocr_words` 消除汉字间多余空格；
//! 5. **独立 MTA 线程执行 WinRT 异步等待**：在 `RO_INIT_MULTITHREADED` 线程内调用 `.get()`，杜绝 Win32 UI 线程 STA 死锁。

#![cfg(target_os = "windows")]

use windows::core::HSTRING;
use windows::Globalization::Language;
use windows::Graphics::Imaging::{
    BitmapAlphaMode, BitmapDecoder, BitmapPixelFormat, SoftwareBitmap,
};
use windows::Media::Ocr::{OcrEngine, OcrResult};
use windows::Security::Cryptography::CryptographicBuffer;
use windows::Storage::Streams::{DataWriter, InMemoryRandomAccessStream};
use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

use crate::postprocess::{clean_cjk_ocr_spaces, join_ocr_words, score_ocr_text};
use crate::preprocess::{
    prepare_high_contrast_for_ocr, prepare_primary_for_ocr, RawBgraImage,
};
use crate::OcrOutcome;

const MISSING_LANG_PACK_HINT: &str = "未检测到可用的 Windows OCR 识别引擎。\n\
请在「系统设置 → 时间和语言 → 语言和区域」中为「中文(简体)」或「English」安装「光学字符识别 (OCR)」功能，\n\
或以管理员运行 PowerShell 执行：\n\
Add-WindowsCapability -Online -Name \"Language.OCR~~~zh-CN~0.0.1.0\"";

/// 对内存中的原始选区位图执行双通道增强 + 多语言择优 OCR。
pub fn recognize_raw_bgra(raw: RawBgraImage) -> Result<OcrOutcome, String> {
    std::thread::spawn(move || {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        recognize_raw_bgra_on_mta(&raw)
    })
    .join()
    .map_err(|_| "Windows OCR 工作线程异常退出".to_string())?
}

/// 对磁盘上的 PNG / BMP / JPEG 图片文件执行 OCR。
pub fn recognize_file(path: &str) -> Result<OcrOutcome, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取截图文件失败: {e}"))?;
    std::thread::spawn(move || {
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
        recognize_encoded_bytes_on_mta(&bytes)
    })
    .join()
    .map_err(|_| "Windows OCR 工作线程异常退出".to_string())?
}

fn recognize_raw_bgra_on_mta(raw: &RawBgraImage) -> Result<OcrOutcome, String> {
    let engines = build_candidate_engines();
    if engines.is_empty() {
        return Ok(OcrOutcome::PermissionDenied(
            MISSING_LANG_PACK_HINT.to_string(),
        ));
    }

    // Pass 1：双线性放大 + 外圈自适应背景色填充边距
    let pass1_img = prepare_primary_for_ocr(raw)?;
    let pass1_bmp = software_bitmap_from_bgra(&pass1_img)?;

    let mut best_text = String::new();
    let mut best_score: i64 = 0;

    if let Ok(text) = run_engine_and_format(&engines[0], &pass1_bmp) {
        let sc = score_ocr_text(&text);
        if sc > best_score {
            best_score = sc;
            best_text = text;
        }
    }

    // 若首选引擎在 Pass 1 已获得充足有效文本（例如 >= 6 分），直接返回以保证极速响应
    if best_score >= 6 {
        return Ok(OcrOutcome::Text(best_text));
    }

    // Pass 2：深色模式自动反相 + 分位数对比度拉伸 + S 曲线锐化
    if let Ok(pass2_img) = prepare_high_contrast_for_ocr(raw) {
        if let Ok(pass2_bmp) = software_bitmap_from_bgra(&pass2_img) {
            if let Ok(text) = run_engine_and_format(&engines[0], &pass2_bmp) {
                let sc = score_ocr_text(&text);
                if sc > best_score {
                    best_score = sc;
                    best_text = text;
                }
            }

            // 若仍为空或得分极低，尝试备用语言引擎（例如用户框选了日文/韩文或纯英文）
            if best_score < 6 {
                for fallback_engine in engines.iter().skip(1).take(2) {
                    for bmp in [&pass1_bmp, &pass2_bmp] {
                        if let Ok(text) = run_engine_and_format(fallback_engine, bmp) {
                            let sc = score_ocr_text(&text);
                            if sc > best_score {
                                best_score = sc;
                                best_text = text;
                            }
                        }
                    }
                    if best_score >= 6 {
                        break;
                    }
                }
            }
        }
    }

    let trimmed = best_text.trim();
    if trimmed.is_empty() {
        Ok(OcrOutcome::EmptyText)
    } else {
        Ok(OcrOutcome::Text(trimmed.to_string()))
    }
}

fn recognize_encoded_bytes_on_mta(bytes: &[u8]) -> Result<OcrOutcome, String> {
    let engines = build_candidate_engines();
    if engines.is_empty() {
        return Ok(OcrOutcome::PermissionDenied(
            MISSING_LANG_PACK_HINT.to_string(),
        ));
    }

    let stream =
        InMemoryRandomAccessStream::new().map_err(|e| format!("创建内存流失败: {e}"))?;
    let output = stream
        .GetOutputStreamAt(0)
        .map_err(|e| format!("获取输出流失败: {e}"))?;
    let writer =
        DataWriter::CreateDataWriter(&output).map_err(|e| format!("创建 DataWriter 失败: {e}"))?;
    writer
        .WriteBytes(bytes)
        .map_err(|e| format!("写入图片字节失败: {e}"))?;
    writer
        .StoreAsync()
        .map_err(|e| format!("存储流数据失败: {e}"))?
        .get()
        .map_err(|e| format!("等待流存储完成失败: {e}"))?;
    stream
        .Seek(0)
        .map_err(|e| format!("重置流偏移失败: {e}"))?;

    let decoder = BitmapDecoder::CreateAsync(&stream)
        .map_err(|e| format!("创建图片解码器失败: {e}"))?
        .get()
        .map_err(|e| format!("解码图片失败: {e}"))?;

    let decoded_bmp = decoder
        .GetSoftwareBitmapAsync()
        .map_err(|e| format!("读取位图失败: {e}"))?
        .get()
        .map_err(|e| format!("等待位图就绪失败: {e}"))?;

    let bgra_bmp = SoftwareBitmap::ConvertWithAlpha(
        &decoded_bmp,
        BitmapPixelFormat::Bgra8,
        BitmapAlphaMode::Ignore,
    )
    .unwrap_or(decoded_bmp);

    let mut best_text = String::new();
    let mut best_score: i64 = 0;

    for engine in engines.iter().take(3) {
        if let Ok(text) = run_engine_and_format(engine, &bgra_bmp) {
            let sc = score_ocr_text(&text);
            if sc > best_score {
                best_score = sc;
                best_text = text;
            }
            if best_score >= 6 {
                break;
            }
        }
    }

    let trimmed = best_text.trim();
    if trimmed.is_empty() {
        Ok(OcrOutcome::EmptyText)
    } else {
        Ok(OcrOutcome::Text(trimmed.to_string()))
    }
}

fn software_bitmap_from_bgra(img: &RawBgraImage) -> Result<SoftwareBitmap, String> {
    let ibuffer = CryptographicBuffer::CreateFromByteArray(&img.bgra)
        .map_err(|e| format!("创建 WinRT 像素缓冲区失败: {e}"))?;
    SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &ibuffer,
        BitmapPixelFormat::Bgra8,
        img.width as i32,
        img.height as i32,
        BitmapAlphaMode::Ignore,
    )
    .map_err(|e| format!("构建 SoftwareBitmap 失败: {e}"))
}

fn try_push_tag(engines: &mut Vec<OcrEngine>, seen_tags: &mut Vec<String>, tag_str: &str) {
    let lower = tag_str.to_ascii_lowercase();
    if seen_tags.contains(&lower) {
        return;
    }
    if let Ok(lang) = Language::CreateLanguage(&HSTRING::from(tag_str)) {
        if let Ok(eng) = OcrEngine::TryCreateFromLanguage(&lang) {
            if let Ok(eng_lang) = eng.RecognizerLanguage() {
                if let Ok(resolved) = eng_lang.LanguageTag() {
                    let resolved_lower = resolved.to_string().to_ascii_lowercase();
                    if !seen_tags.contains(&resolved_lower) {
                        seen_tags.push(resolved_lower);
                        engines.push(eng);
                        return;
                    }
                }
            }
            seen_tags.push(lower);
            engines.push(eng);
        }
    }
}

/// 按优先级构建已安装的 OCR 语言引擎列表：
/// 1. 简体中文 (`zh-Hans-CN` / `zh-Hans` / `zh-CN`) —— 支持中英双语混排，优先级最高
/// 2. 繁体中文 (`zh-Hant-TW` / `zh-Hant-HK` / `zh-Hant`)
/// 3. 用户系统首选语言 (`TryCreateFromUserProfileLanguages`)
/// 4. 英文 / 日文 / 韩文 (`en-US` / `ja` / `ko`)
fn build_candidate_engines() -> Vec<OcrEngine> {
    let mut engines = Vec::new();
    let mut seen_tags = Vec::<String>::new();

    // 1. 显式尝试中英双语能力最强的中文识别包
    for preferred in [
        "zh-Hans-CN",
        "zh-Hans",
        "zh-CN",
        "zh-Hant-TW",
        "zh-Hant-HK",
        "zh-Hant",
    ] {
        try_push_tag(&mut engines, &mut seen_tags, preferred);
    }

    // 2. 枚举系统已安装的识别语言，优先把任何 zh-* 排在前面
    if let Ok(installed) = OcrEngine::AvailableRecognizerLanguages() {
        let count = installed.Size().unwrap_or(0);
        for i in 0..count {
            if let Ok(lang) = installed.GetAt(i) {
                if let Ok(tag) = lang.LanguageTag() {
                    let s = tag.to_string();
                    if s.to_ascii_lowercase().starts_with("zh") {
                        try_push_tag(&mut engines, &mut seen_tags, &s);
                    }
                }
            }
        }
    }

    // 3. 用户系统 Profile 默认语言
    if let Ok(user_eng) = OcrEngine::TryCreateFromUserProfileLanguages() {
        let already_in = user_eng
            .RecognizerLanguage()
            .and_then(|l| l.LanguageTag())
            .map(|t| seen_tags.contains(&t.to_string().to_ascii_lowercase()))
            .unwrap_or(false);
        if !already_in {
            engines.push(user_eng);
        }
    }

    // 4. 其余常用语言包兜底
    for extra in ["en-US", "en-GB", "ja", "ko"] {
        try_push_tag(&mut engines, &mut seen_tags, extra);
    }

    engines
}

fn run_engine_and_format(engine: &OcrEngine, bitmap: &SoftwareBitmap) -> Result<String, String> {
    let result: OcrResult = engine
        .RecognizeAsync(bitmap)
        .map_err(|e| format!("启动 OCR 识别失败: {e}"))?
        .get()
        .map_err(|e| format!("等待 OCR 识别完成失败: {e}"))?;

    let lines = result
        .Lines()
        .map_err(|e| format!("读取 OCR 文本行失败: {e}"))?;
    let line_count = lines.Size().unwrap_or(0);
    if line_count == 0 {
        return Ok(String::new());
    }

    let mut out_lines = Vec::with_capacity(line_count as usize);
    for i in 0..line_count {
        let Ok(line) = lines.GetAt(i) else {
            continue;
        };
        let mut word_pieces = Vec::new();
        if let Ok(words) = line.Words() {
            let word_count = words.Size().unwrap_or(0);
            for w_idx in 0..word_count {
                if let Ok(w) = words.GetAt(w_idx) {
                    if let Ok(t) = w.Text() {
                        let s = t.to_string();
                        if !s.trim().is_empty() {
                            word_pieces.push(s);
                        }
                    }
                }
            }
        }

        let formatted_line = if !word_pieces.is_empty() {
            join_ocr_words(&word_pieces)
        } else if let Ok(raw_line) = line.Text() {
            clean_cjk_ocr_spaces(&raw_line.to_string())
        } else {
            String::new()
        };

        if !formatted_line.is_empty() {
            out_lines.push(formatted_line);
        }
    }

    Ok(out_lines.join("\n"))
}
