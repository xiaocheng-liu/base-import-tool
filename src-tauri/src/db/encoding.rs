//! 错误信息编码规范化
//!
//! # 问题背景
//!
//! 在 Windows 上，达梦（DM）等国产数据库驱动通过 ODBC/JDBC 或原生协议返回的错误信息
//! 经常是 **GBK/GB18030** 编码的本地化字符串（如 "指定的用户名或密码无效"）。
//!
//! 驱动代码使用 `String::from_utf8_lossy` 读取这些字节：
//! - 如果 GBK 双字节**偶然**匹配 UTF-8 的合法序列，会得到"看着像合法"但内容
//!   错乱的字符串；
//! - 如果 GBK 双字节不在合法 UTF-8 范围内，会被替换为 `U+FFFD`（REPLACEMENT CHARACTER）。
//!
//! 这些内容被原样传递到前端 HTML 页面时，浏览器再次用 UTF-8 解码就显示成方块乱码。
//!
//! 在 macOS / Linux 上，达梦驱动返回的错误信息已经是 UTF-8，所以不存在此问题。
//!
//! # 解决思路
//!
//! 整体策略是**保留原字符串**——乱码本身在 Rust 端无法可靠还原原始 GBK 内容（因为
//! `from_utf8_lossy` 已经把原始字节替换为 U+FFFD，信息已经丢失）。但我们可以：
//!
//! 1. 检测出含 `U+FFFD` 的字符串并附上友好的"乱码提示"，让用户知道是编码问题；
//! 2. 提供 [`format_db_error`] 统一格式化函数。

/// 判断字符是否属于 CJK 统一表意文字区段。
fn is_cjk_char(c: char) -> bool {
    matches!(c,
        '\u{4E00}'..='\u{9FFF}'       // CJK Unified Ideographs
        | '\u{3400}'..='\u{4DBF}'      // CJK Unified Ideographs Extension A
        | '\u{F900}'..='\u{FAFF}'      // CJK Compatibility Ideographs
        | '\u{2E80}'..='\u{2EFF}'      // CJK Radicals Supplement
        | '\u{3000}'..='\u{303F}'      // CJK Symbols and Punctuation
    )
}

/// 统计字符串中 CJK 字符数量。
fn count_cjk(s: &str) -> usize {
    s.chars().filter(|c| is_cjk_char(*c)).count()
}

/// 统计字符串中 `U+FFFD` 替换字符的数量。
fn count_replacement(s: &str) -> usize {
    s.chars().filter(|c| *c == '\u{FFFD}').count()
}

/// 规范化数据库错误信息。
///
/// - 已是正常 UTF-8（含 CJK 字符且无 U+FFFD）：原样返回。
/// - 含 U+FFFD：保留原文，并标注「乱码」提示（驱动层错误信息编码问题）。
/// - 其它：原样返回。
pub fn normalize_db_error<E: std::fmt::Display>(err: E) -> String {
    let raw = err.to_string();
    if raw.is_empty() {
        return raw;
    }

    let cjk = count_cjk(&raw);
    let replacement = count_replacement(&raw);

    // 干净 UTF-8 中文，原样返回
    if cjk > 0 && replacement == 0 {
        return raw;
    }

    // 含 U+FFFD：驱动返回的 GBK 字节被 UTF-8 解码时替换为 U+FFFD
    if replacement > 0 {
        return format!(
            "{} [提示：原始错误信息编码异常，已在源头降级为占位符]",
            raw
        );
    }

    raw
}

/// 格式化数据库错误信息：自动转码 + 中文前缀。
///
/// # 示例
/// ```ignore
/// return Err(format_db_error("连接达梦失败", e));
/// ```
pub fn format_db_error<E: std::fmt::Display>(prefix: &str, err: E) -> String {
    let normalized = normalize_db_error(err);
    if prefix.is_empty() {
        normalized
    } else {
        format!("{}: {}", prefix, normalized)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passthrough_utf8_chinese() {
        let s = "达梦数据库连接成功";
        assert_eq!(normalize_db_error(s), s);
    }

    #[test]
    fn passthrough_ascii() {
        let s = "Connection refused";
        assert_eq!(normalize_db_error(s), s);
    }

    #[test]
    fn handles_replacement_chars() {
        // 模拟驱动返回的 GBK 字节被 from_utf8_lossy 替换为 U+FFFD
        let s = "连接达梦失败: \u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}\u{FFFD}";
        let result = normalize_db_error(s);
        assert!(result.contains("连接达梦失败"));
        assert!(result.contains("提示"));
    }

    #[test]
    fn empty_input() {
        assert_eq!(normalize_db_error(""), "");
    }

    #[test]
    fn format_adds_prefix() {
        let result = format_db_error("连接达梦失败", "Connection failed");
        assert_eq!(result, "连接达梦失败: Connection failed");
    }
}
