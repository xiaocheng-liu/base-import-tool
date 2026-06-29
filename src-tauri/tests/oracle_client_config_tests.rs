use base_import_tool_lib::{configure_bundled_oracle_client, has_oracle_library, resolve_oracle_client_dir};
use std::{env, fs};

/// 根据当前平台返回 Oracle 客户端库文件名
fn oracle_lib_name() -> &'static str {
    if cfg!(target_os = "windows") { "oci.dll" }
    else if cfg!(target_os = "linux") { "libclntsh.so" }
    else { "libclntsh.dylib" }
}

/// Oracle Client 配置相关测试。

#[test]
fn configures_oracle_client_dir_only_when_library_exists() {
    let lib = oracle_lib_name();
    let empty_dir = env::temp_dir().join(format!(
        "base-import-tool-empty-oracle-{}",
        std::process::id()
    ));
    let bundled_dir = env::temp_dir().join(format!(
        "base-import-tool-bundled-oracle-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&empty_dir);
    let _ = fs::remove_dir_all(&bundled_dir);
    fs::create_dir_all(&empty_dir).unwrap();
    fs::create_dir_all(&bundled_dir).unwrap();
    fs::write(bundled_dir.join(lib), b"").unwrap();

    env::remove_var("ORACLE_CLIENT_LIB_DIR");
    env::remove_var("ORACLE_HOME");
    configure_bundled_oracle_client(&empty_dir);

    assert!(env::var("ORACLE_CLIENT_LIB_DIR").is_err());

    configure_bundled_oracle_client(&bundled_dir);

    assert_eq!(
        env::var("ORACLE_CLIENT_LIB_DIR").unwrap(),
        bundled_dir.to_string_lossy().to_string()
    );

    env::remove_var("ORACLE_CLIENT_LIB_DIR");
    let _ = fs::remove_dir_all(&empty_dir);
    let _ = fs::remove_dir_all(&bundled_dir);
}

#[test]
fn always_uses_bundled_client_over_system_one() {
    let lib = oracle_lib_name();
    let bundled_dir = env::temp_dir().join(format!(
        "base-import-tool-bundled-priority-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&bundled_dir);
    fs::create_dir_all(&bundled_dir).unwrap();
    fs::write(bundled_dir.join(lib), b"").unwrap();

    // 模拟系统已有 Oracle 环境变量。
    env::set_var("ORACLE_CLIENT_LIB_DIR", "/custom/oracle/client");
    configure_bundled_oracle_client(&bundled_dir);

    // 应该使用打包版本而非系统版本。
    assert_eq!(
        env::var("ORACLE_CLIENT_LIB_DIR").unwrap(),
        bundled_dir.to_string_lossy().to_string()
    );

    env::remove_var("ORACLE_CLIENT_LIB_DIR");
    let _ = fs::remove_dir_all(&bundled_dir);
}

#[test]
fn prefers_dev_oracle_client_dir_when_library_exists() {
    let lib = oracle_lib_name();
    let dev_dir = env::temp_dir().join(format!(
        "base-import-tool-dev-oracle-{}",
        std::process::id()
    ));
    let bundled_dir = env::temp_dir().join(format!(
        "base-import-tool-bundled-oracle-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dev_dir);
    let _ = fs::remove_dir_all(&bundled_dir);
    fs::create_dir_all(&dev_dir).unwrap();
    fs::create_dir_all(&bundled_dir).unwrap();
    fs::write(dev_dir.join(lib), b"").unwrap();
    fs::write(bundled_dir.join(lib), b"").unwrap();

    let resolved = resolve_oracle_client_dir(Some(&dev_dir), Some(&bundled_dir)).unwrap();

    assert_eq!(resolved, dev_dir);

    let _ = fs::remove_dir_all(&dev_dir);
    let _ = fs::remove_dir_all(&bundled_dir);
}

#[test]
fn falls_back_to_bundled_oracle_client_dir_when_dev_library_missing() {
    let lib = oracle_lib_name();
    let dev_dir = env::temp_dir().join(format!(
        "base-import-tool-dev-oracle-missing-{}",
        std::process::id()
    ));
    let bundled_dir = env::temp_dir().join(format!(
        "base-import-tool-bundled-oracle-fallback-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dev_dir);
    let _ = fs::remove_dir_all(&bundled_dir);
    fs::create_dir_all(&dev_dir).unwrap();
    fs::create_dir_all(&bundled_dir).unwrap();
    fs::write(bundled_dir.join(lib), b"").unwrap();

    let resolved = resolve_oracle_client_dir(Some(&dev_dir), Some(&bundled_dir)).unwrap();

    assert_eq!(resolved, bundled_dir);

    let _ = fs::remove_dir_all(&dev_dir);
    let _ = fs::remove_dir_all(&bundled_dir);
}

#[test]
fn has_oracle_library_detects_platform_specific_lib() {
    let lib = oracle_lib_name();
    let dir = env::temp_dir().join(format!(
        "base-import-tool-has-lib-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();

    // 空目录应该返回 false
    assert!(!has_oracle_library(&dir));

    // 放入对应平台的库文件后应该返回 true
    fs::write(dir.join(lib), b"").unwrap();
    assert!(has_oracle_library(&dir));

    let _ = fs::remove_dir_all(&dir);
}
