//! 真正执行 ImageIO + Vision 的 macOS 回归测试，不能只检查 request 属性。

use std::path::Path;
use std::process::Command;
use std::sync::OnceLock;

use objc2::rc::autoreleasepool;
use objc2_core_graphics::CGImage;
use tempfile::TempDir;

use super::{image, vision};

fn fixtures() -> &'static Path {
    static FIXTURES: OnceLock<TempDir> = OnceLock::new();
    FIXTURES
        .get_or_init(|| {
            let directory = TempDir::new().unwrap();
            let script = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("src/ocr/tests/generate-fixtures.swift");
            let output = Command::new("/usr/bin/swift")
                .arg(script)
                .arg(directory.path())
                .output()
                .expect("macOS OCR tests require the Xcode Command Line Tools");
            assert!(
                output.status.success(),
                "生成 OCR 样例失败: {}",
                String::from_utf8_lossy(&output.stderr)
            );
            directory
        })
        .path()
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(fixtures().join(format!("{name}.png"))).unwrap()
}

#[test]
fn native_ocr_reads_real_png_formats_and_single_line_captures() {
    for name in ["english", "single-line", "grayscale", "grayscale-16bit", "transparent"] {
        let text = vision::recognize_bytes(&fixture(name))
            .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(text.contains("Hello OCR"), "{name}: {text:?}");
    }
    let text = vision::recognize_bytes(&fixture("chinese")).unwrap();
    let compact: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    assert!(compact.contains("你好世界"), "chinese: {text:?}");
    assert!(text.contains("Hello OCR"), "mixed Chinese/English: {text:?}");
    if objc2::available!(macos = 13.0) {
        let text = vision::recognize_bytes(&fixture("japanese")).unwrap();
        assert!(text.contains("テキスト"), "japanese: {text:?}");
    }
}

#[test]
fn imageio_decodes_16bit_png_and_normalizes_pixels_before_vision() {
    autoreleasepool(|_| {
        let bytes = fixture("grayscale-16bit");
        assert_eq!(bytes[24], 16, "fixture must actually have 16-bit PNG samples");
        let image = image::prepare_png(&bytes).unwrap();
        assert_eq!(CGImage::bits_per_component(Some(&image)), 8);
        assert_eq!(CGImage::bits_per_pixel(Some(&image)), 32);
        assert!(CGImage::height(Some(&image)) >= 128);
    });
}

#[test]
fn compatibility_retry_really_recognizes_text_on_cpu() {
    autoreleasepool(|_| {
        let image = image::prepare_png(&fixture("english")).unwrap();
        let text = vision::recognize_once(&image, true).unwrap();
        assert!(text.contains("Hello OCR"), "CPU retry: {text:?}");
    });
}
