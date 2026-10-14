//! 图像预处理：解码 PNG 并以截图边缘自适应背景色填充边距，生成 8-bit sRGB `CGImage`。
//!
//! 关键改进：
//! 旧版在 <128px 的小截图四周填充纯白 `(1.0, 1.0, 1.0)`，导致深色模式（白字黑底）单行截图
//! 产生刺眼的白框伪影，干扰 Vision 行检测。新版自动采样原图四周边缘像素的平均色进行无缝填充。

#![cfg(target_os = "macos")]

use objc2_core_foundation::{CFData, CFRetained, CGPoint, CGRect, CGSize};
use objc2_core_graphics::{
    kCGColorSpaceSRGB, CGBitmapContextCreate, CGBitmapContextCreateImage, CGColorSpace, CGContext,
    CGImage, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_image_io::CGImageSource;

const MIN_DIMENSION: usize = 128;
const BORDER: usize = 16;
const MAX_PIXELS: usize = 32_000_000;

pub fn prepare_png(bytes: &[u8]) -> Result<CFRetained<CGImage>, String> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.len() < 24 {
        return Err("截图不是有效的 PNG 图片，请重新框选。".to_string());
    }
    if &bytes[12..16] != b"IHDR" {
        return Err("截图 PNG 缺少图片尺寸信息，请重新框选。".to_string());
    }
    let width = u32::from_be_bytes(bytes[16..20].try_into().unwrap()) as usize;
    let height = u32::from_be_bytes(bytes[20..24].try_into().unwrap()) as usize;
    canvas_size(width, height)?;

    let data = CFData::from_bytes(bytes);
    let source = unsafe { CGImageSource::with_data(&data, None) }
        .ok_or_else(|| "系统无法读取截图 PNG，请重新框选。".to_string())?;
    let image = unsafe { source.image_at_index(0, None) }
        .ok_or_else(|| "截图 PNG 解码失败，请重新框选。".to_string())?;
    let width = CGImage::width(Some(&image));
    let height = CGImage::height(Some(&image));
    let (canvas_width, canvas_height) = canvas_size(width, height)?;

    let color_space = CGColorSpace::with_name(Some(unsafe { kCGColorSpaceSRGB }))
        .ok_or_else(|| "创建 OCR 色彩空间失败".to_string())?;
    let bitmap_info = CGImageAlphaInfo::NoneSkipLast.0 | CGImageByteOrderInfo::Order32Big.0;

    // 先将原图解码到一块小缓冲区以采样四周边缘背景色，确保深色/浅色截图都能平滑延展背景
    let (bg_r, bg_g, bg_b) = sample_border_rgb(&image, width, height, &color_space, bitmap_info);

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

    CGContext::set_rgb_fill_color(Some(&context), bg_r, bg_g, bg_b, 1.0);
    CGContext::fill_rect(Some(&context), rect(0, 0, canvas_width, canvas_height));
    CGContext::draw_image(
        Some(&context),
        rect(
            (canvas_width - width) / 2,
            (canvas_height - height) / 2,
            width,
            height,
        ),
        Some(&image),
    );
    CGBitmapContextCreateImage(Some(&context)).ok_or_else(|| "生成 OCR 位图失败".to_string())
}

/// 采样原图外圈像素的平均 RGB 值（0.0..=1.0），失败时回退白色。
fn sample_border_rgb(
    image: &CGImage,
    width: usize,
    height: usize,
    color_space: &CGColorSpace,
    bitmap_info: u32,
) -> (f64, f64, f64) {
    let mut buf = vec![255u8; width * height * 4];
    let Some(ctx) = (unsafe {
        CGBitmapContextCreate(
            buf.as_mut_ptr().cast(),
            width,
            height,
            8,
            width * 4,
            Some(color_space),
            bitmap_info,
        )
    }) else {
        return (1.0, 1.0, 1.0);
    };
    CGContext::set_rgb_fill_color(Some(&ctx), 1.0, 1.0, 1.0, 1.0);
    CGContext::fill_rect(Some(&ctx), rect(0, 0, width, height));
    CGContext::draw_image(Some(&ctx), rect(0, 0, width, height), Some(image));

    let mut r_sum: u64 = 0;
    let mut g_sum: u64 = 0;
    let mut b_sum: u64 = 0;
    let mut count: u64 = 0;

    let mut add_pixel = |x: usize, y: usize| {
        let idx = (y * width + x) * 4;
        r_sum += buf[idx] as u64;
        g_sum += buf[idx + 1] as u64;
        b_sum += buf[idx + 2] as u64;
        count += 1;
    };

    for x in 0..width {
        add_pixel(x, 0);
        if height > 1 {
            add_pixel(x, height - 1);
        }
    }
    if height > 2 {
        for y in 1..(height - 1) {
            add_pixel(0, y);
            if width > 1 {
                add_pixel(width - 1, y);
            }
        }
    }

    if count == 0 {
        return (1.0, 1.0, 1.0);
    }
    (
        (r_sum as f64 / count as f64) / 255.0,
        (g_sum as f64 / count as f64) / 255.0,
        (b_sum as f64 / count as f64) / 255.0,
    )
}

pub fn canvas_size(width: usize, height: usize) -> Result<(usize, usize), String> {
    if width == 0 || height == 0 {
        return Err("截图尺寸为空，请重新框选。".to_string());
    }
    let padded = width
        .checked_add(BORDER * 2)
        .zip(height.checked_add(BORDER * 2))
        .map(|(w, h)| (w.max(MIN_DIMENSION), h.max(MIN_DIMENSION)))
        .filter(|(w, h)| w.checked_mul(*h).is_some_and(|pixels| pixels <= MAX_PIXELS));
    padded.ok_or_else(|| "截图区域过大，请缩小框选范围后重试。".to_string())
}

fn rect(x: usize, y: usize, width: usize, height: usize) -> CGRect {
    CGRect {
        origin: CGPoint {
            x: x as _,
            y: y as _,
        },
        size: CGSize {
            width: width as _,
            height: height as _,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_small_captures_to_minimum_dimension() {
        assert_eq!(canvas_size(320, 24).unwrap(), (352, 128));
        assert_eq!(canvas_size(8, 8).unwrap(), (128, 128));
        assert!(canvas_size(0, 10).is_err());
    }
}
