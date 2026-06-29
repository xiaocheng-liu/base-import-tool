use base_import_tool_lib::db::oracle_conn::{format_oracle_connect_error, resolve_oracle_client_lib_dir};
use std::fs;
use std::path::PathBuf;

/// 根据当前平台返回 Oracle 客户端库文件名
fn oracle_lib_name() -> &'static str {
    if cfg!(target_os = "windows") { "oci.dll" }
    else if cfg!(target_os = "linux") { "libclntsh.so" }
    else { "libclntsh.dylib" }
}

#[test]
fn uses_oracle_home_root_when_instant_client_library_is_in_root() {
    let root = std::env::temp_dir().join(format!(
        "base-import-tool-oracle-root-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join(oracle_lib_name()), b"").unwrap();

    let dir = resolve_oracle_client_lib_dir(None, Some(root.to_str().unwrap()));

    assert_eq!(dir, Some(PathBuf::from(&root)));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn uses_oracle_home_lib_when_full_client_library_is_under_lib() {
    let root = std::env::temp_dir().join(format!(
        "base-import-tool-oracle-home-{}",
        std::process::id()
    ));
    let lib = root.join("lib");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&lib).unwrap();
    fs::write(lib.join(oracle_lib_name()), b"").unwrap();

    let dir = resolve_oracle_client_lib_dir(None, Some(root.to_str().unwrap()));

    assert_eq!(dir, Some(PathBuf::from(&lib)));

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn formats_oracle_auth_protocol_error_with_actionable_hint() {
    let message = format_oracle_connect_error(
        "OCI Error: ORA-28041: authentication protocol internal error",
    );

    assert!(message.contains("ORA-28041"));
    assert!(message.contains("认证协议不兼容"));
    assert!(message.contains("SQLNET.ALLOWED_LOGON_VERSION"));
    assert!(message.contains("Oracle Instant Client"));
}
