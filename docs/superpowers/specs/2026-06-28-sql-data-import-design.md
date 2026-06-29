# SQL 数据文件导入设计

## 目标

在现有“数据导入”能力上增加对按表拆分 `.sql` 数据文件的支持，使工具可以直接执行类似 `cbs_sys_user.sql`、`clin_wkst_cw_app_menu.sql` 的数据脚本。

## 现状

- 数据导入页当前只扫描并导入 `.csv` 文件。
- `.csv` 文件通过父目录名识别目标库，例如 `kbe/drug_list.csv`。
- 后端已有跨库统一的 `execute_raw_sql` 能力，可直接执行原始 SQL。

## 输入文件规则

### CSV

- 保持现有规则不变。
- 继续通过父目录名识别目标库。

### SQL

- 新增扫描目录下的 `.sql` 文件。
- `.sql` 文件名使用 `<target_db>_<table_name>.sql` 规则识别目标库和展示表名。
- 目标库优先匹配已知库名：`cbs`、`clin_wkst`、`drug_spec`、`his`、`inpt`、`kb_docs`、`kbe`、`outpt`、`procure`。
- 例如：
  - `cbs_sys_user.sql` -> `target_db = cbs`，`table_name = sys_user`
  - `clin_wkst_cw_app_menu.sql` -> `target_db = clin_wkst`，`table_name = cw_app_menu`

## 执行行为

- `.csv` 文件继续走现有批量插入逻辑。
- `.sql` 文件直接读取文件内容并调用数据库连接的 `execute_raw_sql` 执行。
- 所有导入任务继续串行执行，避免脚本之间存在依赖时出现乱序问题。
- `.sql` 文件不校验目标表是否已存在，因为文件内容可能不是单纯插入。

## 界面行为

- 文件扫描页同时展示 `.csv` 和 `.sql` 文件。
- 文件卡片增加文件类型标识。
- 统计文案从“CSV 文件”调整为“导入文件”。
- 任务列表对 `.sql` 文件显示“SQL 执行中/已完成/失败”，不显示行数进度条。

## 错误处理

- 文件名不符合规则的 `.sql` 文件不纳入导入列表。
- `.sql` 文件读取失败时，任务标记失败并记录错误信息。
- `execute_raw_sql` 执行失败时，任务标记失败并保留数据库返回信息。

## 测试范围

- 扫描目录时可识别合法 `.sql` 文件。
- 扫描目录时忽略无法识别目标库的 `.sql` 文件。
- `.sql` 文件名可正确拆分目标库和表名。
- `.sql` 文件导入任务可正确读取文件内容并执行。
- 前端展示可区分 `.csv` 与 `.sql` 两种文件。
