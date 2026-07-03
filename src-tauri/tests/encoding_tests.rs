use base_import_tool_lib::db::encoding::{format_db_error, normalize_db_error};

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
