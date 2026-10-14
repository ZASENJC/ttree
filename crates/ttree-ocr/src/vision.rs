//! macOS Apple Vision (`VNRecognizeTextRequest`) 原生文字识别。

#![cfg(target_os = "macos")]

use objc2::rc::{autoreleasepool, Retained};
use objc2::runtime::AnyObject;
use objc2::{AnyThread, ClassType};
use objc2_core_graphics::CGImage;
use objc2_core_image::{kCIContextUseSoftwareRenderer, CIContext};
use objc2_core_ml::MLCPUComputeDevice;
use objc2_foundation::{NSArray, NSDictionary, NSNumber, NSObjectProtocol, NSString};
use objc2_vision::{
    VNImageOptionCIContext, VNImageRequestHandler, VNRecognizeTextRequest,
    VNRequestTextRecognitionLevel,
};

use crate::OcrOutcome;

pub fn recognize_file(path: &str) -> Result<OcrOutcome, String> {
    let bytes = std::fs::read(path).map_err(|e| format!("读取截图失败: {e}"))?;
    recognize_bytes(&bytes)
}

pub fn recognize_bytes(bytes: &[u8]) -> Result<OcrOutcome, String> {
    autoreleasepool(|_| {
        let image = super::image::prepare_png(bytes)?;
        let first_error = match recognize_once(&image, false) {
            Ok(text) if !text.trim().is_empty() => return Ok(OcrOutcome::Text(text)),
            Ok(_) => None,
            Err(error) => Some(error),
        };

        // 兼容回退：切换软件 CIContext + CPU 计算设备 + revision 2 再试一次
        match recognize_once(&image, true) {
            Ok(text) if !text.trim().is_empty() => Ok(OcrOutcome::Text(text)),
            Ok(_) => Ok(OcrOutcome::EmptyText),
            Err(second_error) => match first_error {
                Some(first) => Err(format!("OCR 识别及兼容重试失败: {first}; {second_error}")),
                None => Ok(OcrOutcome::EmptyText),
            },
        }
    })
}

pub(crate) fn recognize_once(image: &CGImage, compatibility: bool) -> Result<String, String> {
    unsafe {
        let request: Retained<VNRecognizeTextRequest> = VNRecognizeTextRequest::new();
        configure_request(&request, compatibility)?;

        let options = if compatibility || objc2::available!(macos = 27.0) {
            let software = NSNumber::numberWithBool(true);
            let values: [&AnyObject; 1] = [&software];
            let context_options =
                NSDictionary::from_slices(&[kCIContextUseSoftwareRenderer], &values);
            let context = CIContext::contextWithOptions(Some(&context_options));
            let values: [&AnyObject; 1] = [&context];
            NSDictionary::from_slices(&[VNImageOptionCIContext], &values)
        } else {
            NSDictionary::new()
        };

        let handler = VNImageRequestHandler::initWithCGImage_options(
            VNImageRequestHandler::alloc(),
            image,
            &options,
        );

        let requests = NSArray::from_retained_slice(&[Retained::into_super(Retained::into_super(
            request.clone(),
        ))]);
        handler.performRequests_error(&requests).map_err(|e| {
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

    let revision = if !compatibility && objc2::available!(macos = 13.0) {
        3
    } else {
        2
    };
    unsafe {
        request.setRevision(revision);
        // 关键修复：不再将 preferBackgroundProcessing 设为 true，保证菜单栏隐藏窗口状态下拥有前台级识别优先级
        request.setPreferBackgroundProcessing(false);
    }
    if objc2::available!(macos = 13.0) {
        request.setAutomaticallyDetectsLanguage(!compatibility);
    }

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
    let languages: Vec<Retained<NSString>> = preferred_languages(&supported_codes, revision)
        .into_iter()
        .map(NSString::from_str)
        .collect();
    if !languages.is_empty() {
        request.setRecognitionLanguages(&NSArray::from_retained_slice(&languages));
    }

    if compatibility || objc2::available!(macos = 27.0) {
        configure_cpu(request)?;
    }
    Ok(())
}

fn configure_cpu(request: &VNRecognizeTextRequest) -> Result<(), String> {
    unsafe {
        if objc2::available!(macos = 14.0) {
            let stages = request
                .supportedComputeStageDevicesAndReturnError()
                .map_err(|error| format!("读取 OCR 计算设备失败: {error}"))?;
            let mut assigned = false;
            for stage in stages.allKeys().iter() {
                if let Some(devices) = stages.objectForKey(&stage) {
                    if let Some(cpu) = devices
                        .iter()
                        .find(|device| device.isKindOfClass(MLCPUComputeDevice::class()))
                    {
                        request.setComputeDevice_forComputeStage(Some(&cpu), &stage);
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

fn preferred_languages(supported: &[String], revision: usize) -> Vec<&str> {
    let candidates: &[&str] = if revision >= 3 {
        &[
            "zh-Hans", "zh-Hant", "en-US", "ja-JP", "ko-KR", "fr-FR", "de-DE", "es-ES", "ru-RU",
        ]
    } else {
        // revision 2 中文模型仅允许与 en-US 组合
        &["zh-Hans", "zh-Hant", "en-US"]
    };
    candidates
        .iter()
        .copied()
        .filter(|code| supported.iter().any(|s| s.as_str() == *code))
        .collect()
}

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
    fn configures_foreground_priority_and_auto_language_detection() {
        let request = VNRecognizeTextRequest::new();
        configure_request(&request, false).unwrap();
        assert!(unsafe { !request.preferBackgroundProcessing() });
        if objc2::available!(macos = 13.0) {
            assert!(request.automaticallyDetectsLanguage());
        }
    }
}
