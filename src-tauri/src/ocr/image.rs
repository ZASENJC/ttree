//! 将系统 PNG 解码并绘制成 Vision 可以稳定读取的 8-bit sRGB 位图。

#![cfg(target_os = "macos")]

use objc2_core_foundation::{CFData, CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    kCGColorSpaceSRGB, CGBitmapContextCreate, CGBitmapContextCreateImage, CGBitmapInfo,
    CGColorSpace, CGContext, CGImage, CGImageAlphaInfo,
};
use objc2_image_io::CGImageSource;

const MIN_DIMENSION: usize = 128;
const BORDER: usize = 16;
const MAX_PIXELS: usize = 32_000_000;

pub(super) fn prepare_png(bytes: &[u8]) -> Result<CFRetained<CGImage>, String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() < 24 {
        return Err("截图不是有效的 PNG 图片，请重新框选。".to_string());
    }
    // 在原生解码前限制 IHDR 尺寸，避免异常文件触发巨量分配。
    if &bytes[12..16] != b"IHDR" {
        return Err("截图 PNG 缺少图片尺寸信息，请重新框选。".to_string());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap()) as usize;
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize;
    canvas_size(width, height)?;

    let data = CFData::from_bytes(bytes);
    // SAFETY: 使用 ImageIO 默认选项，data 的内容在整个解码期间不变。
    let source = unsafe { CGImageSource::with_data(&data, None) }
        .ok_or_else(|| "系统无法读取截图 PNG，请重新框选。".to_string())?;
    let image = unsafe { source.image_at_index(0, None) }
        .ok_or_else(|| "截图 PNG 解码失败，请重新框选。".to_string())?;
    let width = CGImage::width(Some(&image));
    let height = CGImage::height(Some(&image));
    let (canvas_width, canvas_height) = canvas_size(width, height)?;

    // 显示器截图可能包含灰度、透明度、广色域或 16-bit 像素。统一为
    // 8-bit sRGB、32-bit RGBX，并用白色背景消除透明通道的歧义。
    let color_space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
        .ok_or_else(|| "创建 OCR 色彩空间失败".to_string())?;
    let bitmap_info = CGImageAlphaInfo::NoneSkipLast.0 | CGBitmapInfo::ByteOrder32Big.bits();
    // SAFETY: null 让 CoreGraphics 自行管理内存；尺寸已检查且每像素 4 字节。
    let context = unsafe {
        CGBitmapContextCreate(
            std::ptr::null_mut(),
            canvas_width,
            canvas_height,
            8,
            canvas_width * 4,
            Some(&color_space),
            bitmap_info,
        )
    }
    .ok_or_else(|| "创建 OCR 位图失败".to_string())?;
    CGContext::set_rgb_fill_color(Some(&context), 1.0, 1.0, 1.0, 1.0);
    CGContext::fill_rect(Some(&context), rect(0, 0, canvas_width, canvas_height));
    // 保持原始像素大小；窄单行截图加边距，避免极小图像进入文字模型。
    CGContext::draw_image(
        Some(&context),
        rect((canvas_width - width) / 2, (canvas_height - height) / 2, width, height),
        Some(&image),
    );
    CGBitmapContextCreateImage(Some(&context))
        .ok_or_else(|| "生成 OCR 位图失败".to_string())
}

fn canvas_size(width: usize, height: usize) -> Result<(usize, usize), String> {
    if width == 0 || height == 0 {
        return Err("截图尺寸为空，请重新框选。".to_string());
    }
    let padded = width
        .checked_add(BORDER * 2)
        .zip(height.checked_add(BORDER * 2))
        .map(|(width, height)| (width.max(MIN_DIMENSION), height.max(MIN_DIMENSION)))
        .filter(|(width, height)| {
            width.checked_mul(*height).is_some_and(|pixels| pixels <= MAX_PIXELS)
        });
    padded.ok_or_else(|| "截图区域过大，请缩小框选范围后重试。".to_string())
}

fn rect(x: usize, y: usize, width: usize, height: usize) -> CGRect {
    CGRect {
        origin: CGPoint { x: x as _, y: y as _ },
        size: CGSize { width: width as _, height: height as _ },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_single_line_captures_without_stretching_text() {
        assert_eq!(canvas_size(400, 20).unwrap(), (432, 128));
        assert_eq!(canvas_size(1, 1).unwrap(), (128, 128));
    }

    #[test]
    fn refuses_zero_or_excessive_dimensions() {
        assert!(canvas_size(0, 20).is_err());
        assert!(canvas_size(20, 0).is_err());
        assert!(canvas_size(usize::MAX, 20).is_err());
        assert!(canvas_size(10_000, 10_000).is_err());
    }

    #[test]
    fn rejects_truncated_png_without_entering_vision() {
        assert!(prepare_png(b"\x89PNG\r\n\x1a\n").is_err());
    }
}
