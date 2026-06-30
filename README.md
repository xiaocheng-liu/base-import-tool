# base-import-tool — 数据库导入工具

基于 **Tauri 2** 构建的桌面应用程序，用于将 Oracle 数据库导出的 CSV/SQL 数据文件导入到多种目标数据库（Oracle、MySQL、PostgreSQL、达梦 DM8）。

面向医疗/临床信息系统（PDMS）数据迁移场景，支持 9 个目标业务库：cbs、clin_wkst、drug_spec、his、inpt、kb_docs、kbe、outpt、procure。

## 技术栈

| 层级 | 技术 |
|------|------|
| 桌面框架 | [Tauri 2](https://tauri.app/) |
| 前端 | React 19 + TypeScript + Vite 5 |
| 后端 | Rust 2021 Edition + Tokio |
| 数据库 | oracle · sqlx (MySQL) · tokio-postgres · odbc-api (达梦) |

## 功能

### 1. 数据库连接配置

- 支持四种数据库：Oracle、PostgreSQL、MySQL、达梦 DM8
- 配置持久化到本地 JSON 文件
- 连接测试（5 秒超时提示）

### 2. 表结构初始化（DDL 迁移）

- 选择 Oracle DDL 脚本目录（`*_tables.sql` / `*_indexes.sql`）
- **DDL 自动转换**：Oracle → 目标数据库
  - 类型映射（`NUMBER` → `BIGINT`/`INT`/`DECIMAL`，`VARCHAR2` → `VARCHAR`，`CLOB` → `LONGTEXT`/`TEXT`）
  - 注释迁移（表级、列级）
  - 增量升级（仅新增缺失字段、扩容字段长度）
  - 索引增删改（自动处理 MySQL 索引前缀限制、函数表达式索引过滤）
- 实时执行日志

### 3. 数据导入

- 自动扫描文件夹，按父目录名或文件命名规则识别目标库和表
- 支持 **CSV** 和 **SQL** 两种格式
- 可选"导入前清空目标表"（TRUNCATE，失败自动降级 DELETE）
- 最多 **5 并发**异步导入
- CSV：逐行参数化插入（500 行一批）
- SQL：按分号拆分逐条执行，自动转换 Oracle 函数
  - `TO_DATE` → `STR_TO_DATE`（MySQL）/ `TO_TIMESTAMP`（PostgreSQL）
  - 自动适配标识符引号风格
  - MySQL 自动添加 `ON DUPLICATE KEY UPDATE` 和库前缀
- 实时进度轮询（500ms 间隔）
- 日志复制和导出
- MySQL 常见错误自动给出中文解决建议

## 架构

```
src/                              # React 前端
├── App.tsx                       # 三标签页布局
├── components/
│   ├── DbConfigManager.tsx       # 连接配置管理
│   ├── SchemaInit.tsx            # DDL 初始化
│   ├── FileScanner.tsx           # 文件扫描与预览
│   └── ImportManager.tsx         # 数据导入管理

src-tauri/src/
├── lib.rs                        # 应用入口 & Oracle 客户端配置
├── commands.rs                   # Tauri 命令（核心业务逻辑）
├── models.rs                     # 数据模型
├── config_store.rs               # 配置持久化
├── csv_parser.rs                 # 文件扫描
├── ddl_converter.rs              # DDL 类型转换引擎
└── db/
    ├── mod.rs                    # DbConnection trait
    ├── oracle_conn.rs            # Oracle 实现
    ├── mysql_conn.rs             # MySQL 实现
    ├── pg_conn.rs                # PostgreSQL 实现
    └── dm_conn.rs                # 达梦 DM8 ODBC 实现
```

## 开发

### 环境要求

- Node.js 20+
- Rust 工具链
- Oracle Instant Client（macOS 已内置 ARM64 版本）
- unixODBC（达梦数据库需要）

### 本地运行

```bash
npm install
npm run tauri dev
```

### 构建

```bash
npm run tauri build
```

## 发版

发布基于 Git tag 触发 CI 自动构建：

```bash
git tag v0.1.0
git push origin v0.1.0
```

CI 会：
1. 从 tag 自动同步版本号到 `tauri.conf.json` 和 `Cargo.toml`
2. 并行构建 Windows（msi/nsis）、macOS x64（dmg）、macOS ARM64（dmg）
3. 构建完成后自动创建 GitHub Release 并上传所有产物

## 交付说明

- macOS Apple Silicon `.dmg` 安装包，使用 ad-hoc 签名（未公证）
- 安装包内置 Oracle Instant Client for macOS ARM64
- 仅限内部实施使用
