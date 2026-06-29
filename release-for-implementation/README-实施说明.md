# 数据库导入工具实施说明

## 交付文件

- `数据库导入工具_0.1.0_aarch64.dmg`
- 适用架构：macOS Apple Silicon（arm64/aarch64）
- SHA-256：`f51244d557aa558a5acd80930e93e10aff207f2551f894e755180cb6a43c9e0e`

## 安装方式

1. 双击打开 `数据库导入工具_0.1.0_aarch64.dmg`。
2. 将 `数据库导入工具.app` 拖入 `Applications`。
3. 首次打开如提示来自未认证开发者，可在“系统设置 > 隐私与安全性”中允许打开，或右键应用选择“打开”。

## Oracle 客户端说明

安装包已内置 macOS Apple Silicon 版 Oracle Instant Client，正常使用 Oracle 连接时不需要在实施机器额外安装客户端。

如需临时覆盖内置客户端，可配置环境变量：

```bash
export ORACLE_CLIENT_LIB_DIR=/opt/oracle/instantclient_23_3
```

如果使用完整 Oracle Client，也可配置：

```bash
export ORACLE_HOME=/opt/oracle/product/client
```

程序会优先使用安装包内置客户端；如内置客户端不可用，再读取 `ORACLE_CLIENT_LIB_DIR`，其次读取 `ORACLE_HOME/lib`。

## 使用说明

导入 CSV 时，选择包含以下子目录的根文件夹：

- `KBE`
- `DRUG_SPEC`
- `KB_DOCS`

程序按子目录识别目标库，按 CSV 文件名识别表名，不再兼容旧的文件名前缀格式。

## 签名状态

当前包为内部实施交付包，使用 ad-hoc 签名，未做 Apple Developer ID 签名和公证。
