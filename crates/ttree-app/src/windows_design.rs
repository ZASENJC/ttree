//! Logical-pixel layout shared by the Windows renderer and host-side layout tests.
//! Mirrors the macOS panel: 46px toolbar, four edge-to-edge panes, 128px settings rail.

pub const TOOLBAR: i32 = 46;
#[cfg(target_os = "windows")]
pub const SIDEBAR: i32 = 128;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct BoxRect {
    pub x: i32,
    pub y: i32,
    pub w: i32,
    pub h: i32,
}

impl BoxRect {
    pub const fn new(x: i32, y: i32, w: i32, h: i32) -> Self {
        Self { x, y, w, h }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct TranslationLayout {
    pub source: BoxRect,
    pub languages: BoxRect,
    pub web: BoxRect,
    pub ai: BoxRect,
}

impl TranslationLayout {
    pub fn new(w: i32, h: i32, source: i32, languages: i32, web: i32, ai: bool) -> Self {
        let body = (h - TOOLBAR).max(1);
        let lang_h = languages.clamp(30, 56).min(body);
        // Preserve usable results when a saved large source pane meets a smaller window.
        let result_min = if ai { 152 } else { 76 };
        let source_h = source
            .clamp(68, 240)
            .min((body - lang_h - result_min).max(1));
        let results_y = TOOLBAR + source_h + lang_h;
        let available = (h - results_y).max(1);
        let web_h = if ai {
            web.clamp(76, (available - 76).max(76)).min(available)
        } else {
            available
        };
        Self {
            source: BoxRect::new(0, TOOLBAR, w, source_h),
            languages: BoxRect::new(0, TOOLBAR + source_h, w, lang_h),
            web: BoxRect::new(0, results_y, w, web_h),
            ai: BoxRect::new(
                0,
                results_y + web_h,
                w,
                if ai { available - web_h } else { 0 },
            ),
        }
    }
}

pub fn physical(logical: i32, dpi: u32) -> i32 {
    ((logical as i64 * dpi as i64 + logical.signum() as i64 * 48) / 96) as i32
}

pub fn logical(pixels: i32, dpi: u32) -> i32 {
    ((pixels as i64 * 96 + pixels.signum() as i64 * dpi as i64 / 2) / dpi.max(1) as i64) as i32
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn panes_fit_at_all_supported_sizes_and_saved_extremes() {
        for w in [380, 480, 880] {
            for h in [228, 304, 508, 1142] {
                for source in [68, 104, 240] {
                    for enabled in [false, true] {
                        let l = TranslationLayout::new(w, h, source, 34, 400, enabled);
                        assert_eq!(l.source.y + l.source.h, l.languages.y);
                        assert_eq!(l.languages.y + l.languages.h, l.web.y);
                        assert_eq!(l.web.y + l.web.h, l.ai.y);
                        assert_eq!(l.ai.y + l.ai.h, h);
                        assert!(l.source.h > 0 && l.web.h > 0 && l.ai.h >= 0);
                        if enabled && h >= 304 {
                            assert!(l.web.h >= 76 && l.ai.h >= 76);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn dpi_coordinates_round_trip_and_keep_negative_scroll_offsets() {
        for dpi in [96, 120, 144, 192, 240] {
            for value in [-300, -32, 0, 12, 46, 114, 480, 880] {
                assert!((logical(physical(value, dpi), dpi) - value).abs() <= 1);
            }
        }
    }
}
