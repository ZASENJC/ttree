//! macOS 原生 OCR —— 通过 Apple Vision 框架的 VNRecognizeTextRequest 识别图片文字。
//!
//! 仅 macOS。PNG → ImageIO 解码 → 8-bit RGB CGImage → Vision 同步 perform →
//! 读取 VNRecognizedTextObservation 的 topCandidates(1) 拼接。

#![cfg(target_os = "macos")]

use objc2::rc::{autoreleasepool, Retained};
use objc2::{AnyThread, ClassType};
use objc2_core_graphics::CGImage;
use objc2_core_ml::MLCPUComputeDevice;
use objc2_foundation::{NSArray, NSObjectProtocol, NSString};
use objc2_vision::{VNImageRequestHandler, VNRecognizeTextRequest, VNRequestTextRecognitionLevel};

/// 对指定图片文件路径执行 OCR，返回识别文本（按行拼接）。
pub fn recognize_file(path: &str) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取截图失败: {e}"))?;
    recognize_bytes(&bytes)
}

/// 对内存中的图片字节执行 OCR。
pub fn recognize_bytes(bytes: &[u8]) -> Result<String, String> {
    // Tokio 阻塞线程没有 AppKit 事件循环的自动释放池；每次识别单独释放
    // Vision 创建的临时原生对象，防止多次截图后内存和计算资源累积。
    autoreleasepool(|_| {
        let image = super::image::prepare_png(bytes)?;
        let first_error = match recognize_once(&image, false) {
            Ok(text) if !text.trim().is_empty() => return Ok(text),
            Ok(_) => "未识别到文字".to_string(),
            Err(error) => error,
        };
        // 复用同一截图，但创建全新的 request/handler。CPU 回退避开临时
        // GPU/Metal 故障，关闭语言自动检测和纠错以覆盖模型判断失败。
        match recognize_once(&image, true) {
            Ok(text) if !text.trim().is_empty() => Ok(text),
            Ok(_) => Err("截图中未识别到文字，请框选清晰的文字区域后重试。".to_string()),
            Err(error) => Err(format!("OCR 识别及兼容重试失败: {first_error}; {error}")),
        }
    })
}

pub(super) fn recognize_once(image: &CGImage, compatibility: bool) -> Result<String, String> {
    // SAFETY: Vision API 须在持有有效图像数据期间调用；以下对象均由我们创建并在本函数内存活。
    unsafe {
        // 创建并配置多语言文字识别请求。
        let request: Retained<VNRecognizeTextRequest> = VNRecognizeTextRequest::new();
        configure_request(&request, compatibility)?;

        // 直接提供已规范化的 CGImage，避免 Vision 再走压缩数据解码路径。
        let handler = VNImageRequestHandler::initWithCGImage_options(
            VNImageRequestHandler::alloc(),
            image,
            &objc2_foundation::NSDictionary::new(),
        );

        let requests = NSArray::from_retained_slice(&[Retained::into_super(Retained::into_super(
            request.clone(),
        ))]);
        handler
            .performRequests_error(&requests)
            .map_err(|e| {
                format!(
                    "OCR 执行失败: {e}（图像 {}×{}，{} bit）",
                    CGImage::width(Some(image)),
                    CGImage::height(Some(image)),
                    CGImage::bits_per_component(Some(image)),
                )
            })?;

        Ok(collect_text(&request))
    }
}

fn configure_request(request: &VNRecognizeTextRequest, compatibility: bool) -> Result<(), String> {
    request.setRecognitionLevel(VNRequestTextRecognitionLevel::Accurate);
    request.setUsesLanguageCorrection(!compatibility);

    // 自动检测需要 revision 3，不能只开启属性而依赖系统默认 revision。
    let revision = if !compatibility && objc2::available!(macos = 13.0) {
        3
    } else {
        2
    };
    // 兼容重试使用 revision 2，避开新版识别模型与图像读取器的同一失败路径。
    // SAFETY: revision 2 在最低支持版本 macOS 11 可用；revision 3 经版本判断。
    unsafe {
        request.setRevision(revision);
        request.setPreferBackgroundProcessing(true);
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
    // 新系统后台 OCR 不能依赖 ANE 自动选择。旧 usesCPUOnly 属性在新
    // Vision 上不足以保证 CPU 执行，必须按模型支持的计算阶段显式选择。
    if compatibility || objc2::available!(macos = 27.0) {
        configure_cpu(request)?;
    }
    Ok(())
}

fn configure_cpu(request: &VNRecognizeTextRequest) -> Result<(), String> {
    // SAFETY: 新计算设备 API 和 MLCPUComputeDevice 从 macOS 14 才可用。
    unsafe {
        if objc2::available!(macos = 14.0) {
            let stages = request
                .supportedComputeStageDevicesAndReturnError()
                .map_err(|error| format!("读取 OCR 计算设备失败: {error}"))?;
            let mut assigned = false;
            for stage in stages.allKeys().iter() {
                if let Some(devices) = stages.objectForKey(stage) {
                    if let Some(cpu) = devices
                        .iter()
                        .find(|device| device.isKindOfClass(MLCPUComputeDevice::class()))
                    {
                        request.setComputeDevice_forComputeStage(Some(cpu), stage);
                        assigned = true;
                    }
                }
            }
            if !assigned {
                return Err("此 OCR 模型没有可用的 CPU 计算设备".to_string());
            }
        } else {
            #[allow(deprecated)]
            request.setUsesCPUOnly(true);
        }
    }
    Ok(())
}

fn preferred_languages(supported: &[String]) -> Vec<&str> {
    // Apple 的语言支持查询只说明各语言能单独识别。中文模型只能与
    // 英文配对，不能混入日语、韩语或欧洲语言；自动检测负责选择脚本。
    ["zh-Hans", "zh-Hant", "en-US"]
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
        assert_eq!(unsafe { request.revision() }, 2);
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
    fn supported_languages_do_not_form_an_invalid_chinese_combination() {
        let supported = ["zh-Hans", "zh-Hant", "en-US", "ja-JP", "ko-KR", "fr-FR"]
            .map(String::from);
        assert_eq!(preferred_languages(&supported), ["zh-Hans", "zh-Hant", "en-US"]);
    }

    #[test]
    fn rejects_invalid_image_data_before_starting_vision() {
        assert!(recognize_bytes(&[]).unwrap_err().contains("PNG"));
        assert!(recognize_bytes(b"not an image").is_err());
    }
}
