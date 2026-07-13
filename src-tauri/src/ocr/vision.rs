//! macOS 原生 OCR —— 通过 Apple Vision 框架的 VNRecognizeTextRequest 识别图片文字。
//!
//! 仅 macOS。加载 PNG 为 NSData → VNImageRequestHandler → 同步 perform →
//! 读取 VNRecognizedTextObservation 的 topCandidates(1) 拼接。

#![cfg(target_os = "macos")]

use objc2::rc::Retained;
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
    // SAFETY: Vision API 须在持有有效图像数据期间调用；以下对象均由我们创建并在本函数内存活。
    unsafe {
        let data = NSData::with_bytes(bytes);

        // 创建并配置多语言文字识别请求。
        let request: Retained<VNRecognizeTextRequest> = VNRecognizeTextRequest::new();
        configure_request(&request);

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

fn configure_request(request: &VNRecognizeTextRequest) {
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(true);

    if objc2::available!(macos = 13.0) {
        request.setAutomaticallyDetectsLanguage(true);
    }

    let langs = NSArray::from_retained_slice(&[
        NSString::from_str("zh-Hans"),
        NSString::from_str("zh-Hant"),
        NSString::from_str("en-US"),
        NSString::from_str("ja-JP"),
    ]);
    request.setRecognitionLanguages(&langs);
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
        configure_request(&request);

        if objc2::available!(macos = 13.0) {
            assert!(request.automaticallyDetectsLanguage());
        }
    }
}
