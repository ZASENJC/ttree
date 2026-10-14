//! 语言列表与自适应目标方向判定（中 ⇄ 英）。

pub const ADAPTIVE_TARGET: &str = "adaptive";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LanguageOption {
    pub code: &'static str,
    pub label: &'static str,
}

pub const SOURCE_LANGUAGES: &[LanguageOption] = &[
    LanguageOption {
        code: "auto",
        label: "自动检测",
    },
    LanguageOption {
        code: "zh-CN",
        label: "简体中文",
    },
    LanguageOption {
        code: "zh-TW",
        label: "繁体中文",
    },
    LanguageOption {
        code: "en",
        label: "英语",
    },
    LanguageOption {
        code: "ja",
        label: "日语",
    },
    LanguageOption {
        code: "ko",
        label: "韩语",
    },
    LanguageOption {
        code: "fr",
        label: "法语",
    },
    LanguageOption {
        code: "de",
        label: "德语",
    },
    LanguageOption {
        code: "es",
        label: "西班牙语",
    },
    LanguageOption {
        code: "ru",
        label: "俄语",
    },
];

pub const TARGET_LANGUAGES: &[LanguageOption] = &[
    LanguageOption {
        code: ADAPTIVE_TARGET,
        label: "中 ⇄ 英（自适应）",
    },
    LanguageOption {
        code: "zh-CN",
        label: "简体中文",
    },
    LanguageOption {
        code: "zh-TW",
        label: "繁体中文",
    },
    LanguageOption {
        code: "en",
        label: "英语",
    },
    LanguageOption {
        code: "ja",
        label: "日语",
    },
    LanguageOption {
        code: "ko",
        label: "韩语",
    },
    LanguageOption {
        code: "fr",
        label: "法语",
    },
    LanguageOption {
        code: "de",
        label: "德语",
    },
    LanguageOption {
        code: "es",
        label: "西班牙语",
    },
    LanguageOption {
        code: "ru",
        label: "俄语",
    },
];

const SUPPORTED_SOURCE_CODES: &[&str] = &[
    "auto", "zh-CN", "zh-TW", "en", "ja", "ko", "fr", "de", "es", "ru",
];
const SUPPORTED_TARGET_CODES: &[&str] =
    &["zh-CN", "zh-TW", "en", "ja", "ko", "fr", "de", "es", "ru"];

/// 判断字符是否属于 CJK 统一表意文字（含扩展区）。
fn is_han_char(ch: char) -> bool {
    matches!(
        ch as u32,
        0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xF900..=0xFAFF
            | 0x20000..=0x2A6DF
            | 0x2A700..=0x2B73F
            | 0x2B740..=0x2B81F
            | 0x2B820..=0x2CEAF
            | 0x2CEB0..=0x2EBEF
            | 0x30000..=0x3134F
    )
}

/// 根据输入文本自动选择目标语言：包含汉字则译为英语 (`en`)，否则译为简体中文 (`zh-CN`)。
pub fn resolve_adaptive_target(text: &str) -> &'static str {
    if text.chars().any(is_han_char) {
        "en"
    } else {
        "zh-CN"
    }
}

/// 将用户选择的 `(source, target)`（可能含 `adaptive`）解析为实际请求的语言对。
pub fn resolve_language_pair(source: &str, target: &str, text: &str) -> (String, String) {
    if target == ADAPTIVE_TARGET {
        ("auto".to_string(), resolve_adaptive_target(text).to_string())
    } else {
        (source.to_string(), target.to_string())
    }
}

/// 校验实际发送给引擎的语言对是否合法。
pub fn validate_language_pair(source: &str, target: &str) -> Result<(), String> {
    if !SUPPORTED_SOURCE_CODES.contains(&source) {
        return Err(format!("不支持的源语言代码: {source}"));
    }
    if !SUPPORTED_TARGET_CODES.contains(&target) {
        return Err(format!("不支持的目标语言代码: {target}"));
    }
    Ok(())
}

/// 根据目标语言代码返回中文可读名称（供 AI 提示词渲染使用）。
pub fn language_display_name(code: &str) -> &str {
    match code {
        "auto" => "自动检测的源语言",
        "zh-CN" => "简体中文",
        "zh-TW" => "繁体中文",
        "en" => "英语 (English)",
        "ja" => "日语 (Japanese)",
        "ko" => "韩语 (Korean)",
        "fr" => "法语 (French)",
        "de" => "德语 (German)",
        "es" => "西班牙语 (Spanish)",
        "ru" => "俄语 (Russian)",
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn adaptive_target_detects_chinese_and_non_chinese() {
        assert_eq!(resolve_adaptive_target("你好，世界"), "en");
        assert_eq!(resolve_adaptive_target("Hello, world!"), "zh-CN");
        assert_eq!(resolve_adaptive_target("Rust 性能优化"), "en");
        assert_eq!(resolve_adaptive_target("12345 !@#"), "zh-CN");
    }

    #[test]
    fn resolves_adaptive_pair_to_concrete_codes() {
        let (src, tgt) = resolve_language_pair("en", ADAPTIVE_TARGET, "今天天气不错");
        assert_eq!(src, "auto");
        assert_eq!(tgt, "en");

        let (src, tgt) = resolve_language_pair("zh-CN", "ja", "今天天气不错");
        assert_eq!(src, "zh-CN");
        assert_eq!(tgt, "ja");
    }

    #[test]
    fn validates_known_language_codes() {
        assert!(validate_language_pair("auto", "zh-CN").is_ok());
        assert!(validate_language_pair("en", "zh-TW").is_ok());
        assert!(validate_language_pair("en", "auto").is_err());
        assert!(validate_language_pair("unknown", "en").is_err());
    }
}
