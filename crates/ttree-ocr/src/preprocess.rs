//! 跨平台纯 Rust OCR 图像预处理器（专为解决 Windows.Media.Ocr 识别率痛点设计）。
//!
//! 解决的核心问题：
//! 1. **小字/单行截图识别失败**：当框选高度 `< 56px` 或 `< 112px` 时，自动执行 3× / 2× 双线性插值放大（Bilinear Upscale），
//!    使字高落在 `Windows.Media.Ocr` 最佳识别区间（36~64px）；
//! 2. **紧贴边缘吞首尾字**：采样原图外圈像素计算自适应背景色，四周无缝扩展 `32px` 边距并保证最小画布 `192×128`；
//! 3. **深色模式/低对比度灰字漏识**：提供 Pass-2 高对比度归一化（自动检测深色背景并反相为白底黑字 + 5%~95% 分位对比度拉伸 + S 曲线锐化）。

const BORDER_PADDING: u32 = 32;
const MIN_CANVAS_WIDTH: u32 = 192;
const MIN_CANVAS_HEIGHT: u32 = 128;
const MAX_OCR_PIXELS: u64 = 16_000_000;

/// 内存中自上而下（Top-Down）排列的 32-bit BGRA 位图。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawBgraImage {
    pub width: u32,
    pub height: u32,
    pub bgra: Vec<u8>,
}

impl RawBgraImage {
    pub fn new(width: u32, height: u32, bgra: Vec<u8>) -> Result<Self, String> {
        if width == 0 || height == 0 {
            return Err("截图尺寸为空，请重新框选。".to_string());
        }
        let expected = (width as usize)
            .checked_mul(height as usize)
            .and_then(|px| px.checked_mul(4))
            .ok_or_else(|| "截图尺寸溢出".to_string())?;
        if bgra.len() != expected {
            return Err(format!(
                "位图字节数不匹配：期望 {expected}，实际 {}",
                bgra.len()
            ));
        }
        Ok(Self {
            width,
            height,
            bgra,
        })
    }

    /// 裁剪子区域 `[x..x+w, y..y+h]`。
    pub fn crop(&self, x: u32, y: u32, w: u32, h: u32) -> Result<Self, String> {
        if w == 0 || h == 0 {
            return Err("选区尺寸为空".to_string());
        }
        let x2 = x.saturating_add(w).min(self.width);
        let y2 = y.saturating_add(h).min(self.height);
        if x >= x2 || y >= y2 {
            return Err("选区超出屏幕范围".to_string());
        }
        let actual_w = x2 - x;
        let actual_h = y2 - y;
        let mut out = vec![0u8; (actual_w as usize) * (actual_h as usize) * 4];
        let src_stride = (self.width as usize) * 4;
        let dst_stride = (actual_w as usize) * 4;

        for row in 0..actual_h as usize {
            let src_start = (y as usize + row) * src_stride + (x as usize) * 4;
            let dst_start = row * dst_stride;
            out[dst_start..dst_start + dst_stride]
                .copy_from_slice(&self.bgra[src_start..src_start + dst_stride]);
        }

        Ok(Self {
            width: actual_w,
            height: actual_h,
            bgra: out,
        })
    }
}

/// 计算适合 OCR 的放大倍率（1× / 2× / 3×）。
pub fn recommended_upscale_factor(width: u32, height: u32) -> u32 {
    let base_pixels = (width as u64) * (height as u64);
    if (height < 56 || width < 160) && base_pixels * 9 <= MAX_OCR_PIXELS {
        3
    } else if (height < 112 || width < 320) && base_pixels * 4 <= MAX_OCR_PIXELS {
        2
    } else {
        1
    }
}

/// 消除 Windows ClearType LCD 亚像素渲染产生的红/蓝彩边伪影：
/// ClearType 在 100%/125% 缩放下对字体竖直笔画两侧使用红/蓝亚像素抗锯齿（导致 `|R - B|` 显著偏大且 R/G/B 单调递减/递增），
/// 这会让 `Windows.Media.Ocr` 二值化时笔画粘连或断裂。将其还原为以绿色通道为主导的平滑灰度边缘 `(R + 2G + B) / 4`。
pub fn neutralize_cleartype_fringes(src: &RawBgraImage) -> RawBgraImage {
    let mut out = src.bgra.clone();
    for px in out.chunks_exact_mut(4) {
        let b = px[0] as i32;
        let g = px[1] as i32;
        let r = px[2] as i32;
        let rb_diff = (r - b).abs();
        let is_monotonic = (r >= g && g >= b) || (b >= g && g >= r);
        if is_monotonic && rb_diff >= 24 {
            let luma = ((r + 2 * g + b) >> 2) as u8;
            px[0] = luma;
            px[1] = luma;
            px[2] = luma;
        }
    }
    RawBgraImage {
        width: src.width,
        height: src.height,
        bgra: out,
    }
}

/// Pass-1 预处理：消除 ClearType 彩边 + 小图自适应双线性放大 + 外圈背景色无缝填充边距。
pub fn prepare_primary_for_ocr(img: &RawBgraImage) -> Result<RawBgraImage, String> {
    let defringed = neutralize_cleartype_fringes(img);
    let factor = recommended_upscale_factor(defringed.width, defringed.height);
    let scaled = if factor > 1 {
        upscale_bilinear(&defringed, factor)
    } else {
        defringed
    };
    let (bg_b, bg_g, bg_r) = sample_border_bgr(&scaled);
    pad_with_bg(&scaled, bg_b, bg_g, bg_r)
}

/// Pass-2 高对比度增强预处理：针对深色模式（白字黑底）或低对比度灰字，
/// 自动反相为白底深色字并做分位数对比度拉伸 + S 曲线增强。
pub fn prepare_high_contrast_for_ocr(img: &RawBgraImage) -> Result<RawBgraImage, String> {
    let defringed = neutralize_cleartype_fringes(img);
    let factor = recommended_upscale_factor(defringed.width, defringed.height).max(2);
    let factor =
        if (defringed.width as u64) * (defringed.height as u64) * (factor as u64) * (factor as u64)
            <= MAX_OCR_PIXELS
        {
            factor
        } else {
            1
        };
    let scaled = if factor > 1 {
        upscale_bilinear(&defringed, factor)
    } else {
        defringed
    };

    let (bg_b, bg_g, bg_r) = sample_border_bgr(&scaled);
    let bg_luma = rgb_to_luma(bg_r, bg_g, bg_b);
    let invert = bg_luma < 128;

    let pixel_count = (scaled.width as usize) * (scaled.height as usize);
    let mut lumas = Vec::with_capacity(pixel_count);
    let mut hist = [0usize; 256];

    for chunk in scaled.bgra.chunks_exact(4) {
        let mut y = rgb_to_luma(chunk[2], chunk[1], chunk[0]);
        if invert {
            y = 255 - y;
        }
        lumas.push(y);
        hist[y as usize] += 1;
    }

    let low = percentile_from_hist(&hist, pixel_count, 5);
    let high = percentile_from_hist(&hist, pixel_count, 95).max(low.saturating_add(20));
    let span = (high - low).max(1) as f32;

    let mut enhanced = vec![255u8; scaled.bgra.len()];
    for (i, &y) in lumas.iter().enumerate() {
        let norm = ((y as f32 - low as f32) / span).clamp(0.0, 1.0);
        // S 曲线拉开笔画与底色对比度：暗部更黑，亮部纯白
        let curved = if norm > 0.72 {
            1.0
        } else if norm < 0.28 {
            0.0
        } else {
            ((norm - 0.28) / 0.44).clamp(0.0, 1.0)
        };
        let out_v = (curved * 255.0).round() as u8;
        let idx = i * 4;
        enhanced[idx] = out_v;
        enhanced[idx + 1] = out_v;
        enhanced[idx + 2] = out_v;
        enhanced[idx + 3] = 255;
    }

    let normalized_img = RawBgraImage {
        width: scaled.width,
        height: scaled.height,
        bgra: enhanced,
    };
    pad_with_bg(&normalized_img, 255, 255, 255)
}

#[inline]
fn rgb_to_luma(r: u8, g: u8, b: u8) -> u8 {
    ((77u32 * r as u32 + 150u32 * g as u32 + 29u32 * b as u32) >> 8) as u8
}

fn percentile_from_hist(hist: &[usize; 256], total: usize, pct: usize) -> u8 {
    if total == 0 {
        return 0;
    }
    let target = (total * pct) / 100;
    let mut acc = 0usize;
    for (val, &count) in hist.iter().enumerate() {
        acc += count;
        if acc >= target {
            return val as u8;
        }
    }
    255
}

/// 双线性插值放大 BGRA 图像。
pub fn upscale_bilinear(src: &RawBgraImage, factor: u32) -> RawBgraImage {
    if factor <= 1 {
        return src.clone();
    }
    let dst_w = src.width * factor;
    let dst_h = src.height * factor;
    let mut dst = vec![0u8; (dst_w as usize) * (dst_h as usize) * 4];

    let src_w = src.width as usize;
    let src_h = src.height as usize;
    let scale = 1.0 / (factor as f32);

    for dy in 0..dst_h as usize {
        let sy_f = ((dy as f32 + 0.5) * scale - 0.5).max(0.0);
        let y0 = (sy_f.floor() as usize).min(src_h - 1);
        let y1 = (y0 + 1).min(src_h - 1);
        let wy = sy_f - (y0 as f32);

        for dx in 0..dst_w as usize {
            let sx_f = ((dx as f32 + 0.5) * scale - 0.5).max(0.0);
            let x0 = (sx_f.floor() as usize).min(src_w - 1);
            let x1 = (x0 + 1).min(src_w - 1);
            let wx = sx_f - (x0 as f32);

            let i00 = (y0 * src_w + x0) * 4;
            let i10 = (y0 * src_w + x1) * 4;
            let i01 = (y1 * src_w + x0) * 4;
            let i11 = (y1 * src_w + x1) * 4;
            let dst_idx = (dy * (dst_w as usize) + dx) * 4;

            for c in 0..3 {
                let v00 = src.bgra[i00 + c] as f32;
                let v10 = src.bgra[i10 + c] as f32;
                let v01 = src.bgra[i01 + c] as f32;
                let v11 = src.bgra[i11 + c] as f32;
                let top = v00 + (v10 - v00) * wx;
                let bot = v01 + (v11 - v01) * wx;
                let val = top + (bot - top) * wy;
                dst[dst_idx + c] = val.round().clamp(0.0, 255.0) as u8;
            }
            dst[dst_idx + 3] = 255;
        }
    }

    RawBgraImage {
        width: dst_w,
        height: dst_h,
        bgra: dst,
    }
}

/// 采样图像四周边缘像素的平均 `(B, G, R)`。
pub fn sample_border_bgr(img: &RawBgraImage) -> (u8, u8, u8) {
    let w = img.width as usize;
    let h = img.height as usize;
    if w == 0 || h == 0 {
        return (255, 255, 255);
    }

    let mut b_sum: u64 = 0;
    let mut g_sum: u64 = 0;
    let mut r_sum: u64 = 0;
    let mut count: u64 = 0;

    let mut sample = |x: usize, y: usize| {
        let idx = (y * w + x) * 4;
        b_sum += img.bgra[idx] as u64;
        g_sum += img.bgra[idx + 1] as u64;
        r_sum += img.bgra[idx + 2] as u64;
        count += 1;
    };

    for x in 0..w {
        sample(x, 0);
        if h > 1 {
            sample(x, h - 1);
        }
    }
    if h > 2 {
        for y in 1..(h - 1) {
            sample(0, y);
            if w > 1 {
                sample(w - 1, y);
            }
        }
    }

    if count == 0 {
        return (255, 255, 255);
    }
    (
        (b_sum / count) as u8,
        (g_sum / count) as u8,
        (r_sum / count) as u8,
    )
}

fn pad_with_bg(img: &RawBgraImage, bg_b: u8, bg_g: u8, bg_r: u8) -> Result<RawBgraImage, String> {
    let canvas_w = img
        .width
        .checked_add(BORDER_PADDING * 2)
        .map(|w| w.max(MIN_CANVAS_WIDTH))
        .ok_or_else(|| "画布宽度溢出".to_string())?;
    let canvas_h = img
        .height
        .checked_add(BORDER_PADDING * 2)
        .map(|h| h.max(MIN_CANVAS_HEIGHT))
        .ok_or_else(|| "画布高度溢出".to_string())?;

    if (canvas_w as u64) * (canvas_h as u64) > MAX_OCR_PIXELS {
        return Err("截图区域过大，请缩小框选范围后重试。".to_string());
    }

    let total_px = (canvas_w as usize) * (canvas_h as usize);
    let mut out = vec![0u8; total_px * 4];
    for chunk in out.chunks_exact_mut(4) {
        chunk[0] = bg_b;
        chunk[1] = bg_g;
        chunk[2] = bg_r;
        chunk[3] = 255;
    }

    let offset_x = ((canvas_w - img.width) / 2) as usize;
    let offset_y = ((canvas_h - img.height) / 2) as usize;
    let src_stride = (img.width as usize) * 4;
    let dst_stride = (canvas_w as usize) * 4;

    for row in 0..img.height as usize {
        let src_start = row * src_stride;
        let dst_start = (offset_y + row) * dst_stride + offset_x * 4;
        out[dst_start..dst_start + src_stride]
            .copy_from_slice(&img.bgra[src_start..src_start + src_stride]);
    }

    Ok(RawBgraImage {
        width: canvas_w,
        height: canvas_h,
        bgra: out,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn small_single_line_image_is_upscaled_and_padded() {
        let raw = RawBgraImage::new(80, 24, vec![20u8; 80 * 24 * 4]).unwrap();
        assert_eq!(recommended_upscale_factor(80, 24), 3);
        let prepared = prepare_primary_for_ocr(&raw).unwrap();
        assert_eq!(prepared.width, 80 * 3 + BORDER_PADDING * 2);
        assert!(prepared.height >= MIN_CANVAS_HEIGHT);
    }

    #[test]
    fn dark_mode_image_inverts_to_white_background_in_high_contrast_pass() {
        let mut bgra = vec![24u8; 60 * 30 * 4];
        // 在深灰背景 (24,24,24) 中间画几颗浅灰文字像素 (210,210,210)
        for y in 10..20 {
            for x in 20..40 {
                let idx = (y * 60 + x) * 4;
                bgra[idx] = 210;
                bgra[idx + 1] = 210;
                bgra[idx + 2] = 210;
                bgra[idx + 3] = 255;
            }
        }
        let raw = RawBgraImage::new(60, 30, bgra).unwrap();
        let hc = prepare_high_contrast_for_ocr(&raw).unwrap();
        // 外圈填充应为纯白背景 (255, 255, 255)
        assert_eq!(&hc.bgra[0..3], &[255, 255, 255]);
        // 中心文字像素应被反相并拉伸为深黑色 (0, 0, 0)
        let center_idx =
            ((hc.height as usize / 2) * hc.width as usize + (hc.width as usize / 2)) * 4;
        assert_eq!(&hc.bgra[center_idx..center_idx + 3], &[0, 0, 0]);
    }
}
