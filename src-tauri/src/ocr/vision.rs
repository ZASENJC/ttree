//! macOS 原生 OCR —— 通过 Apple Vision 框架的 VNRecognizeTextRequest 识别图片文字。
//!
//! 仅 macOS。加载 PNG 为 NSData → VNImageRequestHandler → 同步 perform →
//! 读取 VNRecognizedTextObservation 的 topCandidates(1) 拼接。

#![cfg(target_os = "macos")]

use objc2::rc::{autoreleasepool, Retained};
use objc2::AnyThread;
use objc2_foundation::{NSArray, NSData, NSString};
use objc2_vision::{VNImageRequestHandler, VNRecognizeTextRequest, VNRequestTextRecognitionLevel};

/// 对指定图片文件路径执行 OCR，返回识别文本（按行拼接）。
pub fn recognize_file(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取截图失败: {e}"))?;
    recognize_bytes(&bytes)
}

/// 对内存中的图片字节执行 OCR。
pub fn recognize_bytes(bytes: &[u8]) -> Result<String, String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err("截图不是有效的 PNG 图片，请重新框选。".to_string());
    }
    // Tokio 阻塞线程没有 AppKit 事件循环的自动释放池；每次识别单独释放
    // Vision 创建的临时原生对象，防止多次截图后内存和计算资源累积。
    autoreleasepool(|_| {
        let first_error = match recognize_once(bytes, false) {
            Ok(text) if !text.trim().is_empty() => return Ok(text),
            Ok(_) => "未识别到文字".to_string(),
            Err(error) => error,
        };
        // 复用同一截图，但创建全新的 request/handler。CPU 回退避开临时
        // GPU/Metal 故障，关闭语言自动检测和纠错以覆盖模型判断失败。
        match recognize_once(bytes, true) {
            Ok(text) if !text.trim().is_empty() => Ok(text),
            Ok(_) => Err("截图中未识别到文字，请框选清晰的文字区域后重试。".to_string()),
            Err(error) => Err(format!("OCR 识别及兼容重试失败: {first_error}; {error}")),
        }
    })
}

fn recognize_once(bytes: &[u8], compatibility: bool) -> Result<String, String> {
    // SAFETY: Vision API 须在持有有效图像数据期间调用；以下对象均由我们创建并在本函数内存活。
    unsafe {
        let data = NSData::with_bytes(bytes);

        // 创建并配置多语言文字识别请求。
        let request: Retained<VNRecognizeTextRequest> = VNRecognizeTextRequest::new();
        configure_request(&request, compatibility)?;

        // 基于图像数据构建 handler 并同步执行
        let handler = VNImageRequestHandler::initWithData_options(
            VNImageRequestHandler::alloc(),
            &data,
            &objc2_foundation::NSDictionary::new(),
        );

        let requests = NSArray::from_retained_slice(&[Retained::into_super(Retained::into_super(
            request.clone(),
        ))]);
        handler
            .performRequests_error(&requests)
            .map_err(|e| format!("OCR 执行失败: {e}"))?;

        Ok(collect_text(&request))
    }
}

fn configure_request(request: &VNRecognizeTextRequest, compatibility: bool) -> Result<(), String> {
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(!compatibility);

    // 自动检测需要 revision 3，不能只开启属性而依赖系统默认 revision。
    let revision = if objc2::available!(macos = 13.0) {
        3
    } else {
        2
    };
    // SAFETY: revision 2 在最低支持版本 macOS 11 可用；revision 3 经版本判断。
    unsafe {
        request.setRevision(revision);
        request.setPreferBackgroundProcessing(true);
        if compatibility {
            // macOS 11 也必须支持回退，因此保留兼容的 CPU-only API。
            #[allow(deprecated)]
            request.setUsesCPUOnly(true);
        }
    }
    if objc2::available!(macos = 13.0) {
        request.setAutomaticallyDetectsLanguage(!compatibility);
    }
    // 语言支持随 macOS 和 revision 变化；传入不支持的 ja-JP 等语言会使
    // 整个请求失败。实例查询从 macOS 12 才存在，11 使用旧的类查询。
    let supported = unsafe {
        if objc2::available!(macos = 12.0) {
            request.supportedRecognitionLanguagesAndReturnError()
        } else {
            #[allow(deprecated)]
            VNRecognizeTextRequest::supportedRecognitionLanguagesForTextRecognitionLevel_revision_error(
                VNRequestTextRecognitionLevel::Accurate,
                revision,
            )
        }
    }
    .map_err(|error| format!("读取 OCR 支持语言失败: {error}"))?;
    let supported_codes: Vec<String> = supported.iter().map(|code| code.to_string()).collect();
    let languages: Vec<Retained<NSString>> = preferred_languages(&supported_codes)
        .into_iter()
        .map(NSString::from_str)
        .collect();
    if !languages.is_empty() {
        request.setRecognitionLanguages(&NSArray::from_retained_slice(&languages));
    }
    Ok(())
}

fn preferred_languages(supported: &[String]) -> Vec<&str> {
    [
        "zh-Hans", "zh-Hant", "en-US", "ja-JP", "ko-KR", "fr-FR", "de-DE", "es-ES", "ru-RU",
    ]
    .into_iter()
    .filter(|code| supported.iter().any(|supported| supported.as_str() == *code))
    .collect()
}

/// 从请求结果收集识别文本。
unsafe fn collect_text(request: &VNRecognizeTextRequest) -> String {
    let Some(observations) = request.results() else {
        return String::new();
    };
    let mut lines: Vec<String> = Vec::new();
    for obs in observations.iter() {
        let candidates = obs.topCandidates(1);
        if let Some(top) = candidates.firstObject() {
            lines.push(top.string().to_string());
        }
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn enables_automatic_language_detection_by_default_when_available() {
        let request = VNRecognizeTextRequest::new();
        configure_request(&request, false).unwrap();

        if objc2::available!(macos = 13.0) {
            assert!(request.automaticallyDetectsLanguage());
        }
    }

    #[test]
    fn only_configures_languages_supported_by_the_current_request() {
        let request = VNRecognizeTextRequest::new();
        configure_request(&request, false).unwrap();
        let supported = unsafe {
            if objc2::available!(macos = 12.0) {
                request
                    .supportedRecognitionLanguagesAndReturnError()
                    .unwrap()
            } else {
                #[allow(deprecated)]
                VNRecognizeTextRequest::supportedRecognitionLanguagesForTextRecognitionLevel_revision_error(
                    request.recognitionLevel(),
                    request.revision(),
                )
                .unwrap()
            }
        };
        for configured in unsafe { request.recognitionLanguages() }.iter() {
            assert!(supported.iter().any(|code| code == configured));
        }
    }

    #[test]
    fn compatibility_retry_disables_automatic_detection_and_correction() {
        let request = VNRecognizeTextRequest::new();
        configure_request(&request, true).unwrap();
        assert!(!request.usesLanguageCorrection());
        if objc2::available!(macos = 13.0) {
            assert!(!request.automaticallyDetectsLanguage());
        }
    }

    #[test]
    fn older_language_sets_exclude_unsupported_japanese() {
        let languages = ["en-US".to_string(), "zh-Hans".to_string()];
        assert_eq!(preferred_languages(&languages), ["zh-Hans", "en-US"]);
        assert!(preferred_languages(&[]).is_empty());
    }

    #[test]
    fn rejects_invalid_image_data_before_starting_vision() {
        assert!(recognize_bytes(&[]).unwrap_err().contains("PNG"));
        assert!(recognize_bytes(b"not an image").is_err());
    }
}
