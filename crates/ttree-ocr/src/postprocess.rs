//! 跨平台 OCR 文本后处理与多语言引擎结果评分器。
//!
//! 解决 `Windows.Media.Ocr` 的两大典型缺陷：
//! 1. **中文字符间被强行插入空格**：`Windows.Media.Ocr` 的 `OcrLine::Text()` 会将每个中文词/字用空格拼起来
//!    （如 `"截 图 翻 译 测 试"`），严重干扰下游机器翻译与大模型理解。本模块通过 `join_ocr_words` 与
//!    `clean_cjk_ocr_spaces` 智能消去 CJK 字符及全角标点之间的伪空格，同时完整保留英文单词间的合法空格；
//! 2. **错选纯英文引擎识别中文产生乱码符号**：提供 `score_ocr_text` 质量评分函数，供多语言引擎回退择优。

/// 判断字符是否属于 CJK 汉字、假名、谚文或全角标点符号。
#[inline]
pub fn is_cjk_or_fullwidth(c: char) -> bool {
    matches!(
        c as u32,
        0x2E80..=0x2FFF // CJK 部首补充、康熙部首
        | 0x3000..=0x303F // CJK 符号和标点
        | 0x3040..=0x30FF // 平假名、片假名
        | 0x3100..=0x312F // 注音符号
        | 0x3130..=0x318F // 谚文兼容字母
        | 0x3200..=0x32FF // 带圈 CJK
        | 0x3400..=0x4DBF // CJK 统一表意文字扩展 A
        | 0x4E00..=0x9FFF // CJK 统一表意文字基本集
        | 0xAC00..=0xD7AF // 韩文音节
        | 0xF900..=0xFAFF // CJK 兼容表意文字
        | 0xFE30..=0xFE4F // CJK 兼容形式
        | 0xFF00..=0xFFEF // 全角 ASCII、全角标点
        | 0x20000..=0x2FA1F // CJK 扩展 B~F
    )
}

/// 将一行内的 OCR 分词序列智能拼接：
/// - 相邻两个 Token 若任意一侧紧邻全角标点，或两侧均为 CJK 字符，则直接无缝拼接；
/// - 英文/数字单词之间保留单个空格。
pub fn join_ocr_words<S: AsRef<str>>(words: &[S]) -> String {
    let mut out = String::new();
    for word in words {
        let piece = word.as_ref().trim();
        if piece.is_empty() {
            continue;
        }
        if let (Some(prev_char), Some(next_char)) = (out.chars().last(), piece.chars().next()) {
            if should_insert_space_between(prev_char, next_char) {
                out.push(' ');
            }
        }
        out.push_str(piece);
    }
    clean_cjk_ocr_spaces(&out)
}

#[inline]
fn should_insert_space_between(left: char, right: char) -> bool {
    if is_cjk_or_fullwidth(left) && is_cjk_or_fullwidth(right) {
        return false;
    }
    // 全角标点与英文/数字之间也不加多余空格
    if is_fullwidth_punctuation(left) || is_fullwidth_punctuation(right) {
        return false;
    }
    // 两侧均为 ASCII 字母/数字，或一侧为 ASCII 单词标点时保留空格
    left.is_ascii_graphic() && right.is_ascii_graphic()
}

#[inline]
fn is_fullwidth_punctuation(c: char) -> bool {
    matches!(
        c,
        '，' | '。'
            | '、'
            | '；'
            | '：'
            | '？'
            | '！'
            | '“'
            | '”'
            | '‘'
            | '’'
            | '（'
            | '）'
            | '【'
            | '】'
            | '《'
            | '》'
            | '—'
            | '…'
            | '·'
    )
}

/// 清理单行字符串中 CJK 字符之间的冗余空格（兼容直接返回 `OcrLine::Text()` 的路径）。
pub fn clean_cjk_ocr_spaces(line: &str) -> String {
    let chars: Vec<char> = line.trim().chars().collect();
    if chars.is_empty() {
        return String::new();
    }

    let mut out = String::with_capacity(line.len());
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        if ch == ' ' || ch == '\t' {
            // 跳过连续空白，检查左右有效字符
            let prev = out.chars().last();
            let mut j = i + 1;
            while j < chars.len() && (chars[j] == ' ' || chars[j] == '\t') {
                j += 1;
            }
            let next = chars.get(j).copied();
            if let (Some(l), Some(r)) = (prev, next) {
                if should_insert_space_between(l, r) {
                    out.push(' ');
                }
            }
            i = j;
        } else {
            out.push(ch);
            i += 1;
        }
    }
    out
}

/// 评估 OCR 识别结果的有效文本质量分（用于多语言引擎或多轮预处理候选择优）。
pub fn score_ocr_text(text: &str) -> i64 {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return 0;
    }
    let mut score: i64 = 0;
    for ch in trimmed.chars() {
        if is_cjk_or_fullwidth(ch) {
            score += 5;
        } else if ch.is_ascii_alphanumeric() {
            score += 3;
        } else if ch.is_alphabetic() {
            score += 3;
        } else if matches!(ch, '.' | ',' | '-' | '_' | '/' | ':' | ';' | '?' | '!' | '\'' | '"') {
            score += 1;
        } else if ch.is_whitespace() {
            // 不加分也不扣分
        } else {
            // 孤立生僻控制符或乱码噪点轻微扣分
            score -= 1;
        }
    }
    score
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_spaces_between_cjk_words_while_preserving_english_spaces() {
        let words = vec!["这", "是", "Windows", "原", "生", "OCR", "Test", "engine", "！"];
        assert_eq!(join_ocr_words(&words), "这是Windows原生OCR Test engine！");
    }

    #[test]
    fn cleans_inline_cjk_spaces() {
        assert_eq!(
            clean_cjk_ocr_spaces("请 安 装  Windows  OCR  语 言 包 ： 中文（简体）"),
            "请安装Windows OCR语言包：中文（简体）"
        );
    }

    #[test]
    fn scores_meaningful_text_higher_than_ocr_noise() {
        let good = score_ocr_text("你好世界 Hello World");
        let noise = score_ocr_text("|| ~ ^ `");
        assert!(good > 20);
        assert!(good > noise);
    }
}
