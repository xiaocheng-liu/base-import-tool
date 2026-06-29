use crate::models::{ColumnInfo, DbConfig, DbType, IndexInfo, SchemaTarget, TableIdentifier};
use std::collections::HashMap;
use std::path::PathBuf;

/// Oracle DDL 到目标数据库 DDL 的转换器
pub struct DdlConverter {
    schema_dir: PathBuf,
}

/// 解析出的表结构
#[derive(Debug, Clone)]
pub struct TableDef {
    pub schema: String,
    pub table_name: String,
    pub columns: Vec<ColumnDef>,
    pub primary_key: Vec<String>,
    pub comment: Option<String>,
    pub column_comments: HashMap<String, String>,
}

#[derive(Debug, Clone)]
pub struct ColumnDef {
    pub name: String,
    pub data_type: String, // Oracle 类型，如 VARCHAR2(255), NUMBER(10,0), CLOB
    pub nullable: bool,
    pub default_value: Option<String>,
}

/// 索引定义
#[derive(Debug, Clone)]
pub struct IndexDef {
    pub schema: String,
    pub table_name: String,
    pub index_name: String,
    pub unique: bool,
    pub columns: Vec<String>,
}

impl IndexDef {
    pub fn is_plain_column_index(&self) -> bool {
        self.columns
            .iter()
            .all(|column| is_plain_column_name(column))
    }
}

impl DdlConverter {
    pub fn new(schema_dir: PathBuf) -> Self {
        DdlConverter { schema_dir }
    }

    /// 读取指定 target_db 的 tables 和 indexes SQL 文件
    pub fn read_schema_files(&self, target_db: &str) -> Result<(String, String), String> {
        let tables_file = self.schema_dir.join(format!("{}_tables.sql", target_db));
        let indexes_file = self.schema_dir.join(format!("{}_indexes.sql", target_db));

        let tables_sql = std::fs::read_to_string(&tables_file)
            .map_err(|e| format!("读取表结构文件失败 ({}): {}", tables_file.display(), e))?;
        let indexes_sql = std::fs::read_to_string(&indexes_file)
            .map_err(|e| format!("读取索引文件失败 ({}): {}", indexes_file.display(), e))?;

        Ok((tables_sql, indexes_sql))
    }

    /// 列出同时存在表结构和索引脚本的初始化库。
    pub fn list_schema_targets(&self) -> Result<Vec<SchemaTarget>, String> {
        let entries = std::fs::read_dir(&self.schema_dir).map_err(|e| {
            format!(
                "读取 Schema 目录失败 ({}): {}",
                self.schema_dir.display(),
                e
            )
        })?;

        let mut targets = Vec::new();

        for entry in entries {
            let entry = entry.map_err(|e| format!("读取 Schema 文件失败: {}", e))?;
            let file_name = entry.file_name().to_string_lossy().to_string();
            let Some(target_name) = file_name.strip_suffix("_tables.sql") else {
                continue;
            };
            let indexes_file = format!("{}_indexes.sql", target_name);
            if self.schema_dir.join(&indexes_file).is_file() {
                targets.push(SchemaTarget {
                    target_db: target_name.to_string(),
                    tables_file: file_name,
                    indexes_file,
                });
            }
        }

        targets.sort_by(|a, b| a.target_db.to_string().cmp(&b.target_db.to_string()));
        Ok(targets)
    }

    /// 获取单个表的字段和注释信息
    pub fn get_table_schema(
        &self,
        target_db: &str,
        table_name: &str,
    ) -> Result<crate::models::TableSchemaInfo, String> {
        let tables_file = self.schema_dir.join(format!("{}_tables.sql", target_db));
        let tables_sql = std::fs::read_to_string(&tables_file)
            .map_err(|e| format!("读取表结构文件失败 ({}): {}", tables_file.display(), e))?;

        let tables = self.parse_tables(&tables_sql)?;

        let table = tables
            .iter()
            .find(|t| t.table_name.eq_ignore_ascii_case(table_name))
            .ok_or_else(|| format!("在 {} 中未找到表 {}", target_db, table_name))?;

        let columns: Vec<crate::models::ColumnWithComment> = table
            .columns
            .iter()
            .map(|col| crate::models::ColumnWithComment {
                name: col.name.clone(),
                data_type: col.data_type.clone(),
                nullable: col.nullable,
                comment: table.column_comments.get(&col.name).cloned(),
            })
            .collect();

        Ok(crate::models::TableSchemaInfo {
            table_name: table.table_name.clone(),
            table_comment: table.comment.clone(),
            columns,
        })
    }

    /// 解析 Oracle CREATE TABLE 语句
    pub fn parse_tables(&self, sql: &str) -> Result<Vec<TableDef>, String> {
        let mut tables = Vec::new();
        let mut remaining: &str = sql;

        while let Some(table_start) = remaining.find("CREATE TABLE ") {
            // 找到 CREATE TABLE 的位置
            let table_section = &remaining[table_start..];

            // 找到对应的分号（CREATE TABLE 结束）
            let mut depth = 0;
            let mut end_pos = 0;

            for (i, ch) in table_section.char_indices() {
                if ch == '(' {
                    depth += 1;
                } else if ch == ')' {
                    depth -= 1;
                } else if ch == ';' && depth == 0 {
                    end_pos = i;
                    break;
                }
            }

            if end_pos == 0 {
                break;
            }

            let create_stmt = &table_section[..end_pos];
            let after_create = &table_section[end_pos + 1..];

            if let Some(table) = self.parse_single_create_table(create_stmt) {
                tables.push(table);
            }

            // 继续解析当前表后面的 ALTER TABLE ... ADD CONSTRAINT ... PRIMARY KEY
            let mut alter_end = 0;
            let next_table_start = after_create
                .find("CREATE TABLE ")
                .unwrap_or(after_create.len());
            let current_table_tail = &after_create[..next_table_start];
            if let Some(alter_start) = current_table_tail.find("ALTER TABLE ") {
                let alter_section = &current_table_tail[alter_start..];
                if alter_section.contains("PRIMARY KEY") {
                    // 找到 ALTER TABLE 后面的分号
                    if let Some(semi_pos) = alter_section.find(';') {
                        let alter_stmt = &alter_section[..=semi_pos];
                        if alter_stmt.contains("PRIMARY KEY")
                            && tables
                                .last()
                                .map(|table| alter_table_matches(alter_stmt, table))
                                .unwrap_or(false)
                        {
                            if let Some(pk_start) = alter_stmt.find("PRIMARY KEY (") {
                                let pk_section = &alter_stmt[pk_start + 13..];
                                if let Some(pk_end) = pk_section.find(')') {
                                    let pk_cols: Vec<String> = pk_section[..pk_end]
                                        .split(',')
                                        .map(|c| c.trim().trim_matches('"').to_string())
                                        .collect();
                                    if let Some(last_table) = tables.last_mut() {
                                        last_table.primary_key = pk_cols;
                                    }
                                }
                            }
                        }
                        alter_end = alter_start + semi_pos + 1;
                    }
                }
            }

            if alter_end > 0 {
                remaining = &after_create[alter_end..];
            } else {
                remaining = after_create;
            }
        }

        // 解析 COMMENT ON 语句
        self.parse_comments(sql, &mut tables);

        Ok(tables)
    }

    fn parse_single_create_table(&self, stmt: &str) -> Option<TableDef> {
        // 提取 schema.table_name: CREATE TABLE "SCHEMA"."TABLE_NAME"
        let schema_table = stmt
            .strip_prefix("CREATE TABLE ")
            .and_then(|s| s.split('(').next())?
            .trim();

        let parts: Vec<&str> = schema_table.split('.').collect();
        let (schema, table_name) = if parts.len() == 2 {
            (parts[0].trim_matches('"'), parts[1].trim_matches('"'))
        } else {
            ("", schema_table.trim_matches('"'))
        };

        // 提取括号内的列定义
        let paren_start = stmt.find('(')?;
        let paren_end = stmt.rfind(')')?;
        let columns_str = &stmt[paren_start + 1..paren_end];

        // 解析每一列
        let mut columns = Vec::new();
        let mut primary_key = Vec::new();

        let col_parts = self.split_column_definitions(columns_str);

        for part in &col_parts {
            let part = part.trim();
            if part.is_empty() {
                continue;
            }

            // 跳过约束定义
            let upper = part.to_uppercase();
            if upper.starts_with("CONSTRAINT ")
                || upper.starts_with("PRIMARY KEY")
                || upper.starts_with("UNIQUE")
                || upper.starts_with("CHECK")
                || upper.starts_with("USING INDEX")
            {
                // 检查是否是 PRIMARY KEY
                if upper.contains("PRIMARY KEY") {
                    primary_key = self.extract_pk_columns(part);
                }
                continue;
            }

            // 解析列定义: "COLUMN_NAME" TYPE ... [NOT NULL] [DEFAULT ...] [ENABLE]
            if let Some(col) = self.parse_column_def(part) {
                columns.push(col);
            }
        }

        Some(TableDef {
            schema: schema.to_string(),
            table_name: table_name.to_string(),
            columns,
            primary_key,
            comment: None,
            column_comments: HashMap::new(),
        })
    }

    fn split_column_definitions(&self, s: &str) -> Vec<String> {
        let mut result = Vec::new();
        let mut current = String::new();
        let mut depth = 0;

        for ch in s.chars() {
            match ch {
                '(' => {
                    depth += 1;
                    current.push(ch);
                }
                ')' => {
                    depth -= 1;
                    current.push(ch);
                }
                ',' if depth == 0 => {
                    result.push(current.clone());
                    current.clear();
                }
                _ => current.push(ch),
            }
        }

        if !current.trim().is_empty() {
            result.push(current);
        }

        result
    }

    fn parse_column_def(&self, part: &str) -> Option<ColumnDef> {
        let part = part.trim();
        let upper = part.to_uppercase();

        // 提取列名（引号包裹的）
        let name_end = if part.starts_with('"') {
            part[1..].find('"').map(|i| i + 2)?
        } else {
            part.find(' ')?
        };

        let name = part[..name_end].trim_matches('"').to_string();
        let rest = part[name_end..].trim();

        // 提取数据类型
        let mut data_type = String::new();
        let mut rest_after_type = "";
        let mut type_depth = 0;
        let mut type_started = false;
        let mut type_ended = false;

        for (i, ch) in rest.char_indices() {
            if !type_started && !ch.is_whitespace() {
                type_started = true;
            }
            if type_started && !type_ended {
                if ch == '(' {
                    type_depth += 1;
                    data_type.push(ch);
                } else if ch == ')' {
                    type_depth -= 1;
                    data_type.push(ch);
                    if type_depth == 0 {
                        // 可能后面还有 CHAR，如 VARCHAR2(200 CHAR)
                        continue;
                    }
                } else if ch == ' ' && type_depth == 0 {
                    // 检查是否是 "CHAR" 后缀
                    let remaining_rest = rest[i..].trim();
                    if remaining_rest.to_uppercase().starts_with("CHAR") {
                        data_type.push(' ');
                        // 加入 CHAR
                        let char_end = remaining_rest
                            .find(|c: char| c.is_whitespace() || c == ',')
                            .unwrap_or(remaining_rest.len());
                        data_type.push_str(&remaining_rest[..char_end]);
                        rest_after_type = &rest[i + char_end..];
                        type_ended = true;
                    } else {
                        rest_after_type = &rest[i..];
                        type_ended = true;
                    }
                } else {
                    data_type.push(ch);
                }
            }
            if type_ended {
                break;
            }
        }

        if !type_ended && !data_type.is_empty() {
            // 类型后面没有空格了，类型就是整个剩余部分
            rest_after_type = "";
        }

        let nullable = !upper.contains("NOT NULL");
        let default_value = self.extract_default_value(rest_after_type);

        Some(ColumnDef {
            name,
            data_type,
            nullable,
            default_value,
        })
    }

    fn extract_default_value(&self, s: &str) -> Option<String> {
        let upper = s.to_uppercase();
        if let Some(pos) = upper.find("DEFAULT ") {
            let val = s[pos + 8..].trim();
            let val = val.split_whitespace().next().unwrap_or("");
            if val.is_empty() || val == "ENABLE" || val == "NOT" {
                None
            } else {
                Some(val.to_string())
            }
        } else {
            None
        }
    }

    fn extract_pk_columns(&self, s: &str) -> Vec<String> {
        if let Some(start) = s.find('(') {
            if let Some(end) = s.rfind(')') {
                return s[start + 1..end]
                    .split(',')
                    .map(|c| c.trim().trim_matches('"').to_string())
                    .collect();
            }
        }
        Vec::new()
    }

    fn parse_comments(&self, sql: &str, tables: &mut [TableDef]) {
        // 解析 COMMENT ON TABLE "SCHEMA"."TABLE" IS 'comment';
        // 和 COMMENT ON COLUMN "SCHEMA"."TABLE"."COLUMN" IS 'comment';
        let mut remaining = sql;
        let mut comment_count = 0u32;
        while let Some(pos) = remaining.find("COMMENT ON ") {
            let section = &remaining[pos..];
            if let Some(semi_pos) = section.find(';') {
                let stmt = &section[..semi_pos];
                let stmt_upper = stmt.to_uppercase();

                if stmt_upper.contains("COMMENT ON TABLE") {
                    // 提取 schema.table
                    if let Some(tbl_start) = stmt.find('"') {
                        let rest = &stmt[tbl_start..];
                        let parts: Vec<&str> = rest.split('"').collect();
                        if parts.len() >= 4 {
                            let schema = parts[1];
                            let table_name = parts[3];
                            // 提取注释
                            if let Some(comment_start) = stmt.find('\'') {
                                let comment_rest = &stmt[comment_start + 1..];
                                if let Some(comment_end) = comment_rest.find('\'') {
                                    let comment = comment_rest[..comment_end].to_string();
                                    for table in tables.iter_mut() {
                                        if table.schema == schema && table.table_name == table_name
                                        {
                                            table.comment = Some(comment.clone());
                                            comment_count += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                } else if stmt_upper.contains("COMMENT ON COLUMN") {
                    if let Some(tbl_start) = stmt.find('"') {
                        let rest = &stmt[tbl_start..];
                        let parts: Vec<&str> = rest.split('"').collect();
                        if parts.len() >= 6 {
                            let schema = parts[1];
                            let table_name = parts[3];
                            let col_name = parts[5];
                            if let Some(comment_start) = stmt.find('\'') {
                                let comment_rest = &stmt[comment_start + 1..];
                                if let Some(comment_end) = comment_rest.find('\'') {
                                    let comment = comment_rest[..comment_end].to_string();
                                    for table in tables.iter_mut() {
                                        if table.schema == schema && table.table_name == table_name
                                        {
                                            table
                                                .column_comments
                                                .insert(col_name.to_string(), comment.clone());
                                            comment_count += 1;
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                remaining = &section[semi_pos + 1..];
            } else {
                break;
            }
        }
        log::info!(
            "parse_comments: 共解析 {} 条注释，涉及 {} 个表",
            comment_count,
            tables.len()
        );
    }

    /// 解析索引 SQL
    pub fn parse_indexes(&self, sql: &str) -> Vec<IndexDef> {
        let mut indexes = Vec::new();
        let mut remaining = sql;

        while let Some(pos) = remaining.find("CREATE ") {
            let section = &remaining[pos..];
            if let Some(semi_pos) = section.find(';') {
                let stmt = &section[..semi_pos];
                let stmt_upper = stmt.to_uppercase();

                let unique = stmt_upper.contains("UNIQUE INDEX");
                let is_index = stmt_upper.contains(" INDEX ");

                if is_index {
                    // CREATE [UNIQUE] INDEX "SCHEMA"."IDX_NAME" ON "SCHEMA"."TABLE" ("COL1", "COL2")
                    let parts: Vec<&str> = stmt.split('"').collect();
                    if parts.len() >= 8 {
                        let schema = parts[1];
                        let index_name = parts[3];
                        let table_name = parts[7];
                        // 提取 ON 表名后的索引列，函数索引里的括号不能当作列清单边界。
                        if let Some(columns) = extract_index_columns_section(stmt) {
                            let cols: Vec<String> = split_index_columns(columns)
                                .into_iter()
                                .map(|c| normalize_index_column(&c))
                                .filter(|c| !c.is_empty())
                                .collect();

                            indexes.push(IndexDef {
                                schema: schema.to_string(),
                                table_name: table_name.to_string(),
                                index_name: index_name.to_string(),
                                unique,
                                columns: cols,
                            });
                        }
                    }
                }

                remaining = &section[semi_pos + 1..];
            } else {
                break;
            }
        }

        indexes
    }

    /// 将 Oracle 类型映射到目标数据库类型
    pub fn map_oracle_type(oracle_type: &str, db_type: &DbType) -> String {
        let upper = oracle_type.to_uppercase().trim().to_string();
        let upper = if upper.ends_with(" CHAR") {
            upper.replace(" CHAR", "")
        } else {
            upper
        };

        // 提取基本类型（去掉精度部分先）
        let base_type = if let Some(paren) = upper.find('(') {
            upper[..paren].trim()
        } else {
            upper.as_str()
        };

        match db_type {
            DbType::MySQL => match base_type {
                "CHAR" => {
                    if let Some(size_num) = parse_type_length(&upper) {
                        if size_num > 16383 {
                            "LONGTEXT".to_string()
                        } else if size_num >= 1000 {
                            "TEXT".to_string()
                        } else if size_num > 255 {
                            format!("VARCHAR({})", size_num)
                        } else {
                            format!("CHAR({})", size_num)
                        }
                    } else {
                        "CHAR(1)".to_string()
                    }
                }
                "VARCHAR2" | "NVARCHAR2" | "NCHAR" => {
                    if let Some(paren_start) = upper.find('(') {
                        let size: String = upper[paren_start..]
                            .chars()
                            .filter(|c| c.is_ascii_digit())
                            .collect();
                        let size_num: u32 = size.parse().unwrap_or(255);
                        if size_num > 16383 {
                            "LONGTEXT".to_string()
                        } else if size_num >= 1000 {
                            "TEXT".to_string()
                        } else {
                            format!("VARCHAR({})", size_num)
                        }
                    } else {
                        "VARCHAR(255)".to_string()
                    }
                }
                "NUMBER" => {
                    if upper == "NUMBER" || upper == "NUMBER(*,0)" {
                        "BIGINT".to_string()
                    } else if upper.contains("NUMBER(") {
                        let inside = upper.trim_start_matches("NUMBER(").trim_end_matches(')');
                        let parts: Vec<&str> = inside.split(',').collect();
                        if parts.len() == 2 {
                            let scale: i32 = parts[1].trim().parse().unwrap_or(0);
                            if scale > 0 {
                                let precision: i32 =
                                    parts[0].trim().replace('*', "38").parse().unwrap_or(10);
                                format!("DECIMAL({}, {})", precision, scale)
                            } else {
                                let precision: i32 =
                                    parts[0].trim().replace('*', "38").parse().unwrap_or(10);
                                // NUMBER(p,0) 映射：根据精度 p 选择合适的整数类型
                                // TINYINT: -128~127, TINYINT UNSIGNED: 0~255
                                // SMALLINT: -32768~32767, SMALLINT UNSIGNED: 0~65535
                                // INT: -2147483648~2147483647
                                if precision <= 2 {
                                    // NUMBER(1,0)~NUMBER(2,0): 最大 99，TINYINT 足够
                                    "TINYINT".to_string()
                                } else if precision <= 4 {
                                    // NUMBER(3,0)~NUMBER(4,0): 最大 9999，SMALLINT 足够
                                    "SMALLINT".to_string()
                                } else if precision <= 9 {
                                    // NUMBER(5,0)~NUMBER(9,0): 最大 999999999，INT 足够
                                    "INT".to_string()
                                } else if precision <= 18 {
                                    // NUMBER(10,0)~NUMBER(18,0): 最大 999999999999999999，BIGINT 足够
                                    "BIGINT".to_string()
                                } else {
                                    // 更大的精度，使用 DECIMAL
                                    format!("DECIMAL({})", precision)
                                }
                            }
                        } else {
                            "BIGINT".to_string()
                        }
                    } else {
                        "BIGINT".to_string()
                    }
                }
                "CLOB" | "NCLOB" => "LONGTEXT".to_string(),
                "DATE" => "DATETIME".to_string(),
                "TIMESTAMP" => "DATETIME".to_string(),
                "BLOB" => "LONGBLOB".to_string(),
                "FLOAT" => "DOUBLE".to_string(),
                "RAW" => "VARBINARY(255)".to_string(),
                _ => oracle_type.to_string(),
            },
            DbType::PostgreSQL => match base_type {
                "VARCHAR2" | "NVARCHAR2" | "NCHAR" => {
                    if let Some(paren_start) = upper.find('(') {
                        let size: String = upper[paren_start..]
                            .chars()
                            .filter(|c| c.is_ascii_digit())
                            .collect();
                        let size_num: u32 = size.parse().unwrap_or(255);
                        if size_num > 10485760 {
                            "TEXT".to_string()
                        } else {
                            format!("VARCHAR({})", size_num)
                        }
                    } else {
                        "VARCHAR(255)".to_string()
                    }
                }
                "NUMBER" => {
                    if upper == "NUMBER" || upper == "NUMBER(*,0)" {
                        "BIGINT".to_string()
                    } else if upper.contains("NUMBER(") {
                        let inside = upper.trim_start_matches("NUMBER(").trim_end_matches(')');
                        let parts: Vec<&str> = inside.split(',').collect();
                        if parts.len() == 2 {
                            let scale: i32 = parts[1].trim().parse().unwrap_or(0);
                            let precision: i32 =
                                parts[0].trim().replace('*', "38").parse().unwrap_or(10);
                            if scale > 0 {
                                format!("NUMERIC({}, {})", precision, scale)
                            } else {
                                if precision <= 4 {
                                    "SMALLINT".to_string()
                                } else if precision <= 9 {
                                    "INT".to_string()
                                } else {
                                    "BIGINT".to_string()
                                }
                            }
                        } else {
                            "BIGINT".to_string()
                        }
                    } else {
                        "BIGINT".to_string()
                    }
                }
                "CLOB" | "NCLOB" => "TEXT".to_string(),
                "DATE" => "TIMESTAMP".to_string(),
                "TIMESTAMP" => "TIMESTAMP".to_string(),
                "BLOB" => "BYTEA".to_string(),
                "FLOAT" => "DOUBLE PRECISION".to_string(),
                "RAW" => "BYTEA".to_string(),
                _ => oracle_type.to_string(),
            },
            DbType::DM => {
                // 达梦兼容 Oracle 类型
                if base_type == "DATE" {
                    "TIMESTAMP".to_string()
                } else {
                    oracle_type.to_string()
                }
            }
            DbType::Oracle => oracle_type.to_string(),
        }
    }

    /// 生成目标数据库的 CREATE TABLE DDL
    pub fn generate_create_table(table: &TableDef, db_type: &DbType) -> String {
        let table_name = match db_type {
            DbType::MySQL => Self::format_table_name(&table.schema, &table.table_name, db_type),
            DbType::PostgreSQL => {
                if table.schema.is_empty() {
                    format!("\"{}\"", table.table_name.to_lowercase())
                } else {
                    format!(
                        "\"{}\".\"{}\"",
                        table.schema.to_lowercase(),
                        table.table_name.to_lowercase()
                    )
                }
            }
            _ => format!("\"{}\"", table.table_name.to_uppercase()),
        };

        let mut col_defs: Vec<String> = Vec::new();

        for col in &table.columns {
            let col_name = match db_type {
                DbType::MySQL => format!("`{}`", col.name.to_lowercase()),
                DbType::PostgreSQL => format!("\"{}\"", col.name.to_lowercase()),
                _ => format!("\"{}\"", col.name.to_uppercase()),
            };

            let col_type = Self::map_oracle_type(&col.data_type, db_type);
            let nullable = if col.nullable { "" } else { " NOT NULL" };

            let default = Self::format_default_value(&col_type, &col.default_value, db_type);

            col_defs.push(format!("{} {}{}{}", col_name, col_type, nullable, default));
        }

        // 主键
        if !table.primary_key.is_empty() {
            let pk_cols: Vec<String> = table
                .primary_key
                .iter()
                .filter(|pk| {
                    table
                        .columns
                        .iter()
                        .any(|c| c.name.eq_ignore_ascii_case(pk))
                })
                .map(|c| match db_type {
                    DbType::MySQL => format!("`{}`", c.to_lowercase()),
                    DbType::PostgreSQL => format!("\"{}\"", c.to_lowercase()),
                    _ => format!("\"{}\"", c.to_uppercase()),
                })
                .collect();
            if !pk_cols.is_empty() {
                col_defs.push(format!("PRIMARY KEY ({})", pk_cols.join(", ")));
            }
        }

        let engine = match db_type {
            DbType::MySQL => " ENGINE=InnoDB DEFAULT CHARSET=utf8mb4".to_string(),
            _ => String::new(),
        };

        format!(
            "CREATE TABLE IF NOT EXISTS {} (\n  {}\n){};",
            table_name,
            col_defs.join(",\n  "),
            engine
        )
    }

    /// 生成表注释 DDL（根据数据库类型）
    pub fn generate_table_comment_ddl(table: &TableDef, db_type: &DbType) -> Option<String> {
        let comment = table.comment.as_ref()?;
        log::info!(
            "generate_table_comment_ddl: {}.{} -> comment={}",
            table.schema,
            table.table_name,
            comment
        );
        if comment.is_empty() {
            return None;
        }
        let table_name = Self::format_table_name(&table.schema, &table.table_name, db_type);
        let escaped_comment = comment.replace('\'', "''");
        match db_type {
            DbType::MySQL => {
                Some(format!(
                    "ALTER TABLE {} COMMENT '{}';",
                    table_name, escaped_comment
                ))
            }
            DbType::DM | DbType::Oracle => {
                Some(format!(
                    "COMMENT ON TABLE {} IS '{}';",
                    table_name, escaped_comment
                ))
            }
            DbType::PostgreSQL => {
                Some(format!(
                    "COMMENT ON TABLE {} IS '{}';",
                    table_name, escaped_comment
                ))
            }
        }
    }

    /// 生成字段注释 DDL（根据数据库类型），返回多条语句
    pub fn generate_column_comment_ddl(table: &TableDef, db_type: &DbType) -> Vec<String> {
        log::info!(
            "generate_column_comment_ddl: {}.{} -> column_comments count={}",
            table.schema,
            table.table_name,
            table.column_comments.len()
        );
        let mut stmts = Vec::new();
        let table_name = Self::format_table_name(&table.schema, &table.table_name, db_type);

        for (col_name, comment) in &table.column_comments {
            if comment.is_empty() {
                continue;
            }
            // 查找该列在 table.columns 中是否存在，并获取其类型信息
            let col_def = table.columns.iter().find(|c| c.name.eq_ignore_ascii_case(col_name));
            let Some(col_def) = col_def else {
                log::warn!(
                    "generate_column_comment_ddl: column {} not found in table {}.{}",
                    col_name, table.schema, table.table_name
                );
                continue;
            };

            let escaped_comment = comment.replace('\'', "''");

            match db_type {
                DbType::MySQL => {
                    // MySQL 需要用 MODIFY COLUMN + 完整定义来设置列注释
                    let formatted_col = format!("`{}`", col_name.to_lowercase());
                    let col_type = Self::map_oracle_type(&col_def.data_type, db_type);
                    // 主键列在 MySQL 中必须是 NOT NULL
                    let is_pk = table.primary_key.iter().any(|pk| pk.eq_ignore_ascii_case(col_name));
                    let nullable = if is_pk || !col_def.nullable { "NOT NULL" } else { "NULL" };
                    let default = Self::format_default_value(&col_type, &col_def.default_value, db_type);
                    stmts.push(format!(
                        "ALTER TABLE {} MODIFY COLUMN {} {} {} {} COMMENT '{}';",
                        table_name, formatted_col, col_type, nullable,
                        if default.is_empty() { "" } else { &default },
                        escaped_comment
                    ));
                }
                DbType::PostgreSQL => {
                    let formatted_col = format!("\"{}\"", col_name.to_lowercase());
                    stmts.push(format!(
                        "COMMENT ON COLUMN {}.{} IS '{}';",
                        table_name, formatted_col, escaped_comment
                    ));
                }
                DbType::Oracle | DbType::DM => {
                    let formatted_col = format!("\"{}\"", col_name.to_uppercase());
                    stmts.push(format!(
                        "COMMENT ON COLUMN {}.{} IS '{}';",
                        table_name, formatted_col, escaped_comment
                    ));
                }
            }
        }
        log::info!(
            "generate_column_comment_ddl: {}.{} -> generated {} DDL statements",
            table.schema,
            table.table_name,
            stmts.len()
        );
        stmts
    }

    /// 生成单个字段的注释 DDL
    pub fn generate_single_column_comment_ddl(
        table: &TableDef,
        col_def: &ColumnDef,
        col_name: &str,
        comment: &str,
        db_type: &DbType,
    ) -> String {
        let table_name = Self::format_table_name(&table.schema, &table.table_name, db_type);
        let escaped_comment = comment.replace('\'', "''");

        match db_type {
            DbType::MySQL => {
                let formatted_col = format!("`{}`", col_name.to_lowercase());
                let col_type = Self::map_oracle_type(&col_def.data_type, db_type);
                let is_pk = table.primary_key.iter().any(|pk| pk.eq_ignore_ascii_case(col_name));
                let nullable = if is_pk || !col_def.nullable { "NOT NULL" } else { "NULL" };
                let default = Self::format_default_value(&col_type, &col_def.default_value, db_type);
                format!(
                    "ALTER TABLE {} MODIFY COLUMN {} {} {} {} COMMENT '{}';",
                    table_name, formatted_col, col_type, nullable,
                    if default.is_empty() { "" } else { &default },
                    escaped_comment
                )
            }
            DbType::PostgreSQL => {
                let formatted_col = format!("\"{}\"", col_name.to_lowercase());
                format!(
                    "COMMENT ON COLUMN {}.{} IS '{}';",
                    table_name, formatted_col, escaped_comment
                )
            }
            DbType::Oracle | DbType::DM => {
                let formatted_col = format!("\"{}\"", col_name.to_uppercase());
                format!(
                    "COMMENT ON COLUMN {}.{} IS '{}';",
                    table_name, formatted_col, escaped_comment
                )
            }
        }
    }

    pub fn generate_create_index_for_table(
        index: &IndexDef,
        table: Option<&TableDef>,
        db_type: &DbType,
    ) -> Option<String> {
        if matches!(db_type, DbType::MySQL) && !index.is_plain_column_index() {
            return None;
        }

        let unique = if index.unique { "UNIQUE " } else { "" };
        let table_name = Self::format_table_name(&index.schema, &index.table_name, db_type);

        let index_name = match db_type {
            DbType::MySQL => format!("`{}`", index.index_name.to_lowercase()),
            DbType::PostgreSQL => format!("\"{}\"", index.index_name.to_lowercase()),
            _ => format!("\"{}\"", index.index_name.to_uppercase()),
        };

        // 计算 MySQL 索引总字节数（utf8mb4 下每字符 4 字节），超出 3072 则需要对 VARCHAR 列加前缀
        let mysql_index_columns: Vec<(String, u32)> = if matches!(db_type, DbType::MySQL) {
            index
                .columns
                .iter()
                .map(|c| {
                    let column_name = c.to_lowercase();
                    let byte_len = get_mysql_column_byte_length(c, table);
                    (column_name, byte_len)
                })
                .collect()
        } else {
            Vec::new()
        };

        let needs_prefix_for_size =
            matches!(db_type, DbType::MySQL)
                && mysql_index_columns.iter().map(|(_, len)| len).sum::<u32>() > 3072;

        let cols: Vec<String> = index
            .columns
            .iter()
            .map(|c| match db_type {
                DbType::MySQL => {
                    let column_name = c.to_lowercase();
                    if should_use_mysql_index_prefix(c, table)
                        || (needs_prefix_for_size
                            && get_mysql_column_byte_length(c, table) > 0
                            && !is_mysql_text_or_blob_column(c, table))
                    {
                        format!("`{}`(191)", column_name)
                    } else {
                        format!("`{}`", column_name)
                    }
                }
                DbType::PostgreSQL => format!("\"{}\"", c.to_lowercase()),
                _ => format!("\"{}\"", c.to_uppercase()),
            })
            .collect();

        Some(format!(
            "CREATE {}INDEX {} ON {} ({});",
            unique,
            index_name,
            table_name,
            cols.join(", ")
        ))
    }

    /// 生成新增字段 DDL。
    pub fn generate_add_column(table: &TableDef, column: &ColumnDef, db_type: &DbType) -> String {
        let table_name = Self::format_table_name(&table.schema, &table.table_name, db_type);
        let column_def = Self::format_column_definition(column, db_type);

        match db_type {
            DbType::MySQL => format!("ALTER TABLE {} ADD COLUMN {};", table_name, column_def),
            _ => format!("ALTER TABLE {} ADD {};", table_name, column_def),
        }
    }

    /// 生成字段长度扩容 DDL。
    pub fn generate_modify_column(
        table: &TableDef,
        column: &ColumnDef,
        db_type: &DbType,
    ) -> String {
        let table_name = Self::format_table_name(&table.schema, &table.table_name, db_type);
        let column_def = Self::format_column_definition(column, db_type);

        match db_type {
            DbType::PostgreSQL => {
                let column_name = Self::format_column_name(&column.name, db_type);
                let column_type = Self::map_oracle_type(&column.data_type, db_type);
                format!(
                    "ALTER TABLE {} ALTER COLUMN {} TYPE {};",
                    table_name, column_name, column_type
                )
            }
            DbType::MySQL => format!("ALTER TABLE {} MODIFY COLUMN {};", table_name, column_def),
            _ => format!("ALTER TABLE {} MODIFY {};", table_name, column_def),
        }
    }

    fn format_table_name(schema: &str, table_name: &str, db_type: &DbType) -> String {
        match db_type {
            DbType::MySQL => {
                if schema.trim().is_empty() {
                    format!("`{}`", table_name.to_lowercase())
                } else {
                    format!(
                        "`{}`.`{}`",
                        schema.to_lowercase(),
                        table_name.to_lowercase()
                    )
                }
            }
            DbType::PostgreSQL => {
                if schema.is_empty() {
                    format!("\"public\".\"{}\"", table_name.to_lowercase())
                } else {
                    format!(
                        "\"{}\".\"{}\"",
                        schema.to_lowercase(),
                        table_name.to_lowercase()
                    )
                }
            }
            _ => format!(
                "\"{}\".\"{}\"",
                schema.to_uppercase(),
                table_name.to_uppercase()
            ),
        }
    }

    fn format_column_name(column_name: &str, db_type: &DbType) -> String {
        match db_type {
            DbType::MySQL => format!("`{}`", column_name.to_lowercase()),
            DbType::PostgreSQL => format!("\"{}\"", column_name.to_lowercase()),
            _ => format!("\"{}\"", column_name.to_uppercase()),
        }
    }

    fn format_column_definition(column: &ColumnDef, db_type: &DbType) -> String {
        let column_name = Self::format_column_name(&column.name, db_type);
        let column_type = Self::map_oracle_type(&column.data_type, db_type);
        let nullable = if column.nullable { "" } else { " NOT NULL" };
        let default = Self::format_default_value(&column_type, &column.default_value, db_type);

        format!("{} {}{}{}", column_name, column_type, nullable, default)
    }

    pub fn format_default_value(
        column_type: &str,
        default_value: &Option<String>,
        db_type: &DbType,
    ) -> String {
        let Some(value) = default_value.as_deref().map(str::trim) else {
            return String::new();
        };

        if value.is_empty() || value.eq_ignore_ascii_case("NULL") {
            return String::new();
        }

        if matches!(db_type, DbType::MySQL) {
            if is_mysql_text_or_blob(column_type) {
                return String::new();
            }
            if value.eq_ignore_ascii_case("SYSDATE") || value.eq_ignore_ascii_case("SYSTIMESTAMP") {
                return " DEFAULT CURRENT_TIMESTAMP".to_string();
            }
            if is_mysql_temporal_type(column_type) && value.trim_matches('\'').is_empty() {
                return String::new();
            }
            if value.contains(".\"") || value.to_uppercase().contains(".NEXTVAL") {
                return String::new();
            }
            if (column_type.starts_with("TINYINT")
                || column_type.starts_with("SMALLINT")
                || column_type.starts_with("INT")
                || column_type.starts_with("BIGINT")
                || column_type.starts_with("DECIMAL"))
                && value.trim_matches('\'').is_empty()
            {
                return String::new();
            }
        }

        if value == "0" || value == "1" || value.parse::<f64>().is_ok() {
            format!(" DEFAULT {}", value)
        } else {
            format!(
                " DEFAULT '{}'",
                value.trim_matches('\'').replace('\'', "''")
            )
        }
    }

    /// 执行 DDL 增量升级（CREATE TABLE + ADD/MODIFY COLUMN + CREATE INDEX）
    pub async fn execute_ddl(
        &self,
        db_config: &DbConfig,
        target_db: &str,
    ) -> Result<String, String> {
        let (tables_sql, indexes_sql) = self.read_schema_files(target_db)?;
        let tables = self.parse_tables(&tables_sql)?;
        let indexes = self.parse_indexes(&indexes_sql);

        let conn = crate::db::create_connection(db_config).await?;

        // 先测试连接
        conn.test_connection().await?;

        let mut results = Vec::new();
        let mut total_created_tables = 0;
        let mut total_added_columns = 0;
        let mut total_modified_columns = 0;
        let mut total_indexes = 0;

        if matches!(db_config.db_type, DbType::MySQL) {
            let mut schemas: Vec<String> = tables
                .iter()
                .map(|table| table.schema.to_lowercase())
                .filter(|schema| !schema.is_empty())
                .collect();
            schemas.sort();
            schemas.dedup();

            for schema in &schemas {
                let ddl = format!(
                    "CREATE DATABASE IF NOT EXISTS `{}` DEFAULT CHARACTER SET utf8mb4 COLLATE utf8mb4_unicode_ci;",
                    schema
                );
                results.push(format!("  SQL: {}", ddl));
                if let Err(e) = conn.execute_raw_sql(&ddl).await {
                    return Err(format!("创建 MySQL 数据库 {} 失败: {}", schema, e));
                }
            }
        }

        // 执行建表或字段增量升级
        for table in &tables {
            let table_id = TableIdentifier {
                schema: table.schema.clone(),
                table_name: table.table_name.clone(),
            };

            let table_is_new = !conn.schema_table_exists(&table_id).await?;
            if table_is_new {
                let ddl = Self::generate_create_table(table, &db_config.db_type);
                results.push(format!(
                    "  ▶ 创建表 {}.{}",
                    table.schema, table.table_name
                ));
                results.push(format!("    SQL: {}", ddl));
                let column_names: Vec<String> = table
                    .columns
                    .iter()
                    .map(|c| c.name.clone())
                    .collect();
                results.push(format!(
                    "    字段({}): {}",
                    column_names.len(),
                    column_names.join(", ")
                ));
                match conn.execute_raw_sql(&ddl).await {
                    Ok(_) => {
                        total_created_tables += 1;
                        results.push(format!(
                            "    ✓ 表 {}.{} 创建成功",
                            table.schema, table.table_name
                        ));
                    }
                    Err(e) => {
                        results.push(format!(
                            "    ✗ 表 {}.{} 创建失败: {}",
                            table.schema, table.table_name, e
                        ));
                        // 建表失败则跳过后续的列操作和注释
                        continue;
                    }
                }
            }

            if !table_is_new {
                let existing_columns = conn.get_columns(&table_id).await?;
                for column in &table.columns {
                    let existing = existing_columns
                        .iter()
                        .find(|c| c.name.eq_ignore_ascii_case(&column.name));

                    match existing {
                        None => {
                            let ddl = Self::generate_add_column(table, column, &db_config.db_type);
                            results.push(format!(
                                "  ▶ 表 {}.{} 新增字段 {}",
                                table.schema, table.table_name, column.name
                            ));
                            results.push(format!("    SQL: {}", ddl));
                            match conn.execute_raw_sql(&ddl).await {
                                Ok(_) => {
                                    total_added_columns += 1;
                                    results.push(format!(
                                        "    ✓ 新增字段 {} 成功",
                                        column.name
                                    ));
                                }
                                Err(e) => {
                                    results.push(format!(
                                        "    ✗ 新增字段 {} 失败: {}",
                                        column.name, e
                                    ));
                                }
                            }
                        }
                        Some(existing) if should_expand_column(existing, column) => {
                            let ddl = Self::generate_modify_column(table, column, &db_config.db_type);
                            let old_type_display = format_existing_column_type(existing);
                            let new_type = Self::map_oracle_type(&column.data_type, &db_config.db_type);
                            results.push(format!(
                                "  ▶ 表 {}.{} 扩容字段 {} ({} → {})",
                                table.schema, table.table_name, column.name,
                                old_type_display, new_type
                            ));
                            results.push(format!("    SQL: {}", ddl));
                            match conn.execute_raw_sql(&ddl).await {
                                Ok(_) => {
                                    total_modified_columns += 1;
                                    results.push(format!(
                                        "    ✓ 扩容字段 {} 成功",
                                        column.name
                                    ));
                                }
                                Err(e) => {
                                    results.push(format!(
                                        "    ✗ 扩容字段 {} 失败: {}",
                                        column.name, e
                                    ));
                                }
                            }
                        }
                        Some(_) => {}
                    }
                }
            }

            // 执行表注释
            if let Some(expected_comment) = &table.comment {
                if !expected_comment.is_empty() {
                    let existing_comment = conn.get_table_comment(&table_id).await.unwrap_or(None);
                    let comment_changed = existing_comment.as_deref() != Some(expected_comment.as_str());

                    if comment_changed {
                        if let Some(table_comment_ddl) =
                            Self::generate_table_comment_ddl(table, &db_config.db_type)
                        {
                            results.push(format!(
                                "  ▶ 表注释 {}.{}: {} → {}",
                                table.schema, table.table_name,
                                existing_comment.as_deref().unwrap_or("(无)"),
                                expected_comment
                            ));
                            results.push(format!("    SQL: {}", table_comment_ddl));
                            match conn.execute_raw_sql(&table_comment_ddl).await {
                                Ok(_) => {
                                    results.push(format!(
                                        "    ✓ 表注释 {}.{} 更新成功",
                                        table.schema, table.table_name
                                    ));
                                }
                                Err(e) => {
                                    results.push(format!(
                                        "    ⚠ 表 {}.{} 注释更新失败: {}",
                                        table.schema, table.table_name, e
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            // 执行字段注释
            if !table.column_comments.is_empty() {
                let existing_comments = conn.get_column_comments(&table_id).await.unwrap_or_default();
                let mut changed_count = 0usize;
                let mut success_count = 0usize;

                for (col_name, expected_comment) in &table.column_comments {
                    if expected_comment.is_empty() {
                        continue;
                    }
                    let existing = existing_comments
                        .iter()
                        .find(|(name, _)| name.eq_ignore_ascii_case(col_name))
                        .map(|(_, c)| c.as_str());
                    let comment_changed = existing != Some(expected_comment.as_str());

                    if comment_changed {
                        changed_count += 1;
                        // 找到该列定义
                        if let Some(col_def) = table.columns.iter().find(|c| c.name.eq_ignore_ascii_case(col_name)) {
                            let ddl = Self::generate_single_column_comment_ddl(
                                table, col_def, col_name, expected_comment, &db_config.db_type,
                            );
                            results.push(format!(
                                "  ▶ 字段注释 {}.{}.{}: {} → {}",
                                table.schema, table.table_name, col_name,
                                existing.unwrap_or("(无)"),
                                expected_comment
                            ));
                            results.push(format!("    SQL: {}", ddl));
                            match conn.execute_raw_sql(&ddl).await {
                                Ok(_) => success_count += 1,
                                Err(e) => {
                                    results.push(format!(
                                        "    ⚠ 字段注释 {} 更新失败: {}",
                                        col_name, e
                                    ));
                                }
                            }
                        }
                    }
                }
                if changed_count > 0 {
                    results.push(format!(
                        "    ✓ 字段注释 {}.{} 更新完成 ({}/{} 个变更)",
                        table.schema, table.table_name, success_count, changed_count
                    ));
                }
            }
        }

        // 按表分组处理索引（新增/修改/删除）
        let tables_in_schema: Vec<&TableDef> = tables.iter().collect();
        let mut table_indexes: HashMap<(&str, &str), Vec<&IndexDef>> = HashMap::new();
        for index in &indexes {
            let key = (index.schema.as_str(), index.table_name.as_str());
            table_indexes.entry(key).or_default().push(index);
        }

        for table in &tables_in_schema {
            let table_id = TableIdentifier {
                schema: table.schema.clone(),
                table_name: table.table_name.clone(),
            };

            let existing_indexes = match conn.get_indexes(&table_id).await {
                Ok(value) => value,
                Err(e) => {
                    results.push(format!(
                        "  ✗ 查询表 {}.{} 索引失败: {}",
                        table.schema, table.table_name, e
                    ));
                    continue;
                }
            };

            let key = (table.schema.as_str(), table.table_name.as_str());
            let expected_indexes = table_indexes.get(&key).map(|v| v.as_slice()).unwrap_or(&[]);

            // 1. 删除多余的索引（数据库中存在但 DDL 中没有的）
            for existing in &existing_indexes {
                // 跳过主键索引
                if existing.name.eq_ignore_ascii_case("PRIMARY") {
                    continue;
                }
                let still_needed = expected_indexes.iter().any(|expected| {
                    expected.index_name.eq_ignore_ascii_case(&existing.name)
                });
                if !still_needed {
                    let drop_ddl = match &db_config.db_type {
                        DbType::MySQL => format!(
                            "DROP INDEX `{}` ON {};",
                            existing.name.to_lowercase(),
                            Self::format_table_name(&table.schema, &table.table_name, &db_config.db_type)
                        ),
                        DbType::PostgreSQL => format!(
                            "DROP INDEX IF EXISTS \"{}\".\"{}\";",
                            table.schema.to_lowercase(),
                            existing.name.to_lowercase()
                        ),
                        _ => format!(
                            "DROP INDEX \"{}\".\"{}\";",
                            table.schema.to_uppercase(),
                            existing.name.to_uppercase()
                        ),
                    };
                    results.push(format!(
                        "  ▶ 删除多余索引 {} (列: {})",
                        existing.name,
                        existing.columns.join(", ")
                    ));
                    results.push(format!("    SQL: {}", drop_ddl));
                    match conn.execute_raw_sql(&drop_ddl).await {
                        Ok(_) => {
                            total_indexes += 1;
                            results.push(format!("    ✓ 索引 {} 已删除", existing.name));
                        }
                        Err(e) => {
                            results.push(format!("    ✗ 删除索引 {} 失败: {}", existing.name, e));
                        }
                    }
                }
            }

            // 2. 新增或修改索引
            for index in expected_indexes {
                let existing_same_name = existing_indexes
                    .iter()
                    .find(|ei| ei.name.eq_ignore_ascii_case(&index.index_name));

                let existing_same_columns = existing_indexes
                    .iter()
                    .find(|ei| same_columns(&ei.columns, &index.columns)
                        && !ei.name.eq_ignore_ascii_case(&index.index_name));

                // 同名索引但列不同 → 删除旧索引后重建
                if let Some(old_index) = existing_same_name {
                    if !same_columns(&old_index.columns, &index.columns) {
                        let drop_ddl = match &db_config.db_type {
                            DbType::MySQL => format!(
                                "DROP INDEX `{}` ON {};",
                                old_index.name.to_lowercase(),
                                Self::format_table_name(&table.schema, &table.table_name, &db_config.db_type)
                            ),
                            DbType::PostgreSQL => format!(
                                "DROP INDEX IF EXISTS \"{}\".\"{}\";",
                                table.schema.to_lowercase(),
                                old_index.name.to_lowercase()
                            ),
                            _ => format!(
                                "DROP INDEX \"{}\".\"{}\";",
                                table.schema.to_uppercase(),
                                old_index.name.to_uppercase()
                            ),
                        };
                        results.push(format!(
                            "  ▶ 修改索引 {}: 列 {} → {}",
                            index.index_name,
                            old_index.columns.join(", "),
                            index.columns.join(", ")
                        ));
                        results.push(format!("    SQL: {}", drop_ddl));
                        match conn.execute_raw_sql(&drop_ddl).await {
                            Ok(_) => {
                                results.push(format!("    ✓ 旧索引 {} 已删除", index.index_name));
                            }
                            Err(e) => {
                                results.push(format!(
                                    "    ✗ 删除旧索引 {} 失败: {}",
                                    index.index_name, e
                                ));
                                continue;
                            }
                        }
                        // 继续创建新索引（走下面的创建逻辑）
                    } else {
                        // 同名同列，索引已存在且未变，跳过
                        continue;
                    }
                } else if existing_same_columns.is_some() {
                    // 不同名但同列，索引已存在，跳过
                    continue;
                }

                // 创建索引
                let Some(ddl) =
                    Self::generate_create_index_for_table(index, Some(table), &db_config.db_type)
                else {
                    results.push(format!(
                        "  ↷ 索引 {} 跳过: MySQL 暂不支持 Oracle 函数表达式索引",
                        index.index_name
                    ));
                    continue;
                };
                results.push(format!(
                    "  ▶ 创建索引 {} (列: {})",
                    index.index_name,
                    index.columns.join(", ")
                ));
                results.push(format!("    SQL: {}", ddl));
                match conn.execute_raw_sql(&ddl).await {
                    Ok(_) => {
                        total_indexes += 1;
                        results.push(format!("    ✓ 索引 {} 创建成功", index.index_name));
                    }
                    Err(e) => {
                        results.push(format!("    ✗ 索引 {} 创建失败: {}", index.index_name, e));
                    }
                }
            }
        }

        results.push(format!(
            "初始化完成：创建 {} 张表，新增 {} 个字段，扩容 {} 个字段，新增/修改 {} 个索引",
            total_created_tables, total_added_columns, total_modified_columns, total_indexes
        ));

        Ok(results.join("\n"))
    }
}

pub fn should_expand_column(existing: &ColumnInfo, expected: &ColumnDef) -> bool {
    let expected_type = expected.data_type.to_uppercase();
    let existing_type = existing.data_type.to_uppercase();

    if is_lob_text_type(&existing_type) {
        return false;
    }

    if is_text_type(&expected_type) {
        let Some(expected_len) = parse_type_length(&expected_type) else {
            return false;
        };
        return existing.data_length.unwrap_or(0) < expected_len;
    }

    if expected_type.starts_with("NUMBER")
        || expected_type.starts_with("DECIMAL")
        || expected_type.starts_with("NUMERIC")
    {
        if has_wildcard_number_precision(&expected_type) {
            return false;
        }

        let Some((expected_precision, expected_scale)) = parse_number_precision(&expected_type)
        else {
            return false;
        };
        let precision_grew = existing.data_precision.unwrap_or(0) < expected_precision;
        let scale_grew = existing.data_scale.unwrap_or(0) < expected_scale;
        return precision_grew || scale_grew;
    }

    false
}

fn is_text_type(data_type: &str) -> bool {
    data_type.contains("CHAR") || data_type.contains("VARCHAR") || data_type.contains("VARCHAR2")
}

fn is_lob_text_type(data_type: &str) -> bool {
    let upper = data_type.to_uppercase();
    upper.contains("TEXT") || upper.contains("CLOB")
}

fn has_wildcard_number_precision(data_type: &str) -> bool {
    data_type
        .find('(')
        .and_then(|start| {
            let value_start = start + 1;
            data_type[value_start..]
                .find(')')
                .map(|end| &data_type[value_start..value_start + end])
        })
        .map(|inside| inside.split(',').next().unwrap_or("").trim() == "*")
        .unwrap_or(false)
}

fn is_mysql_text_or_blob(data_type: &str) -> bool {
    let upper = data_type.to_uppercase();
    upper.contains("TEXT") || upper.contains("BLOB")
}

fn is_mysql_temporal_type(data_type: &str) -> bool {
    let upper = data_type.to_uppercase();
    upper.starts_with("DATE")
        || upper.starts_with("DATETIME")
        || upper.starts_with("TIMESTAMP")
        || upper.starts_with("TIME")
}

fn alter_table_matches(stmt: &str, table: &TableDef) -> bool {
    let parts: Vec<&str> = stmt.split('"').collect();
    if parts.len() < 4 {
        return false;
    }

    parts[1].eq_ignore_ascii_case(&table.schema) && parts[3].eq_ignore_ascii_case(&table.table_name)
}

fn should_use_mysql_index_prefix(column_name: &str, table: Option<&TableDef>) -> bool {
    let Some(table) = table else {
        return false;
    };
    table
        .columns
        .iter()
        .find(|column| column.name.eq_ignore_ascii_case(column_name))
        .map(|column| {
            let mysql_type = DdlConverter::map_oracle_type(&column.data_type, &DbType::MySQL);
            if is_mysql_text_or_blob(&mysql_type) {
                return true;
            }
            // utf8mb4 下每个字符最多 4 字节，InnoDB 索引最大 3072 字节
            // 对于 VARCHAR 列，如果长度 > 768（3072/4），也需要加前缀
            if let Some(len) = parse_varchar_length(&mysql_type) {
                return len > 768;
            }
            false
        })
        .unwrap_or(false)
}

/// 获取 MySQL 列的字节长度（utf8mb4 下每字符 4 字节）
fn get_mysql_column_byte_length(column_name: &str, table: Option<&TableDef>) -> u32 {
    let Some(table) = table else {
        return 0;
    };
    table
        .columns
        .iter()
        .find(|column| column.name.eq_ignore_ascii_case(column_name))
        .map(|column| {
            let mysql_type = DdlConverter::map_oracle_type(&column.data_type, &DbType::MySQL);
            // TEXT/BLOB 类型的列，在索引中已有前缀(191)，按 191*4=764 算
            if is_mysql_text_or_blob(&mysql_type) {
                return 764;
            }
            // VARCHAR(n) 按 n*4 算
            if let Some(len) = parse_varchar_length(&mysql_type) {
                return len * 4;
            }
            // 整数、日期等固定长度类型，按实际占用算
            estimate_fixed_column_bytes(&mysql_type)
        })
        .unwrap_or(0)
}

/// 估算 MySQL 固定长度列的字节数
fn estimate_fixed_column_bytes(mysql_type: &str) -> u32 {
    let upper = mysql_type.to_uppercase();
    if upper.starts_with("TINYINT") {
        1
    } else if upper.starts_with("SMALLINT") {
        2
    } else if upper.starts_with("INT") || upper.starts_with("FLOAT") {
        4
    } else if upper.starts_with("BIGINT") || upper.starts_with("DOUBLE") || upper.starts_with("DATETIME") || upper.starts_with("TIMESTAMP") {
        8
    } else if upper.starts_with("DECIMAL") || upper.starts_with("NUMERIC") {
        // DECIMAL 在索引中通常按实际精度，保守估计 16
        16
    } else if upper.starts_with("DATE") || upper.starts_with("TIME") {
        3
    } else if upper.starts_with("CHAR(") {
        if let Some(len) = parse_varchar_length(&upper) {
            len * 4
        } else {
            4
        }
    } else if upper.starts_with("VARBINARY") || upper.starts_with("BINARY") {
        255
    } else {
        // 未知类型保守估计 255
        255
    }
}

/// 将数据库中已有的列类型格式化为可读的大写字符串（带长度信息）
fn format_existing_column_type(existing: &ColumnInfo) -> String {
    let upper = existing.data_type.to_uppercase();
    let mut display = if let Some(len) = existing.data_length {
        if len > 0 && (upper.starts_with("VARCHAR") || upper.starts_with("CHAR")) {
            format!("{}({})", upper, len)
        } else if upper.starts_with("TEXT") || upper.starts_with("BLOB") {
            // TEXT/BLOB 不需要显示长度，直接返回
            upper.to_string()
        } else if upper.starts_with("DECIMAL") || upper.starts_with("NUMERIC") {
            if let Some(p) = existing.data_precision {
                if let Some(s) = existing.data_scale {
                    format!("{}({}, {})", upper, p, s)
                } else {
                    format!("{}({})", upper, p)
                }
            } else {
                upper.to_string()
            }
        } else {
            upper.to_string()
        }
    } else {
        upper.to_string()
    };

    display
}

/// 检查列是否为 TEXT/BLOB 类型（在 MySQL 映射后）
fn is_mysql_text_or_blob_column(column_name: &str, table: Option<&TableDef>) -> bool {
    let Some(table) = table else {
        return false;
    };
    table
        .columns
        .iter()
        .find(|column| column.name.eq_ignore_ascii_case(column_name))
        .map(|column| {
            let mysql_type = DdlConverter::map_oracle_type(&column.data_type, &DbType::MySQL);
            is_mysql_text_or_blob(&mysql_type)
        })
        .unwrap_or(false)
}

fn parse_varchar_length(data_type: &str) -> Option<u32> {
    let upper = data_type.to_uppercase();
    if !upper.starts_with("VARCHAR(") {
        return None;
    }
    let start = upper.find('(')? + 1;
    let end = upper[start..].find(')')? + start;
    upper[start..end].parse().ok()
}

fn split_index_columns(columns: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let mut in_quote = false;
    let mut previous = '\0';

    for ch in columns.chars() {
        if ch == '"' && previous != '\\' {
            in_quote = !in_quote;
        }

        match ch {
            '(' if !in_quote => {
                depth += 1;
                current.push(ch);
            }
            ')' if !in_quote => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 && !in_quote => {
                result.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
        previous = ch;
    }

    if !current.trim().is_empty() {
        result.push(current.trim().to_string());
    }

    result
}

fn extract_index_columns_section(stmt: &str) -> Option<&str> {
    let stmt_upper = stmt.to_uppercase();
    let on_pos = stmt_upper.find(" ON ")?;
    let after_on = &stmt[on_pos + 4..];
    let mut in_quote = false;
    let mut list_start = None;

    for (offset, ch) in after_on.char_indices() {
        if ch == '"' {
            in_quote = !in_quote;
        }
        if ch == '(' && !in_quote {
            list_start = Some(offset);
            break;
        }
    }

    let start = list_start?;
    let mut depth = 0;
    in_quote = false;

    for (offset, ch) in after_on[start..].char_indices() {
        if ch == '"' {
            in_quote = !in_quote;
        }

        if in_quote {
            continue;
        }

        if ch == '(' {
            depth += 1;
        } else if ch == ')' {
            depth -= 1;
            if depth == 0 {
                let content_start = start + 1;
                let content_end = start + offset;
                return Some(&after_on[content_start..content_end]);
            }
        }
    }

    None
}

fn normalize_index_column(column: &str) -> String {
    let trimmed = column.trim();
    if is_quoted_identifier(trimmed) {
        return trimmed.trim_matches('"').to_string();
    }
    trimmed.to_string()
}

fn is_plain_column_name(column: &str) -> bool {
    !column.contains('(')
        && !column.contains(')')
        && !column.contains('\'')
        && !column.contains(',')
        && !column.contains('"')
}

fn is_quoted_identifier(value: &str) -> bool {
    value.len() >= 2
        && value.starts_with('"')
        && value.ends_with('"')
        && value.matches('"').count() == 2
}

fn parse_type_length(data_type: &str) -> Option<u32> {
    let start = data_type.find('(')? + 1;
    let end = data_type[start..].find(')')? + start;
    data_type[start..end]
        .split_whitespace()
        .next()?
        .parse()
        .ok()
}

fn parse_number_precision(data_type: &str) -> Option<(u32, i32)> {
    let start = data_type.find('(')? + 1;
    let end = data_type[start..].find(')')? + start;
    let parts: Vec<&str> = data_type[start..end].split(',').collect();
    let precision_text = parts.first()?.trim();
    if precision_text == "*" {
        return None;
    }
    let precision = precision_text.parse().ok()?;
    let scale = parts
        .get(1)
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(0);
    Some((precision, scale))
}

fn index_exists(expected: &IndexDef, existing_indexes: &[IndexInfo]) -> bool {
    existing_indexes.iter().any(|index| {
        index.name.eq_ignore_ascii_case(&expected.index_name)
            || same_columns(&index.columns, &expected.columns)
    })
}

fn same_columns(left: &[String], right: &[String]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .all(|(l, r)| l.eq_ignore_ascii_case(r))
}
