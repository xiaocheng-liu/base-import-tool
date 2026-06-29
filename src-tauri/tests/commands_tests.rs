use base_import_tool_lib::commands::{
    find_sql_keyword, import_target_table, prepare_sql_statements, read_csv_data, read_sql_data,
};
use base_import_tool_lib::db::DbValue;
use base_import_tool_lib::models::{CsvFileInfo, DbType, ImportFileType};
use std::fs;

#[test]
fn reads_empty_csv_fields_as_null_values() {
    let file = std::env::temp_dir().join(format!(
        "base-import-tool-null-csv-{}.csv",
        std::process::id()
    ));
    fs::write(&file, "id,kc_order,name\n1,,分类\n").unwrap();

    let data = read_csv_data(file.to_str().unwrap()).unwrap();

    assert_eq!(data.columns, vec!["id", "kc_order", "name"]);
    assert_eq!(
        data.rows[0],
        vec![
            DbValue::Text("1".to_string()),
            DbValue::Null,
            DbValue::Text("分类".to_string())
        ]
    );

    let _ = fs::remove_file(file);
}

#[test]
fn builds_import_target_table_from_csv_database_folder() {
    let csv_file = CsvFileInfo {
        file_name: "attribute_dict.csv".to_string(),
        file_path: "/data/drug_spec/attribute_dict.csv".to_string(),
        file_type: ImportFileType::Csv,
        target_db: "drug_spec".to_string(),
        table_name: "attribute_dict".to_string(),
        row_count: None,
        columns: vec!["id".to_string()],
    };

    let table = import_target_table(&csv_file);

    assert_eq!(table.schema, "drug_spec");
    assert_eq!(table.table_name, "attribute_dict");
}

#[test]
fn reads_sql_file_content_for_import() {
    let file = std::env::temp_dir().join(format!(
        "base-import-tool-import-sql-{}.sql",
        std::process::id()
    ));
    fs::write(&file, "insert into cbs.sys_user(id) values (1);").unwrap();

    let sql = read_sql_data(file.to_str().unwrap()).unwrap();

    assert_eq!(sql, "insert into cbs.sys_user(id) values (1);");

    let _ = fs::remove_file(file);
}

#[test]
fn prepares_mysql_sql_statements_from_oracle_style_script() {
    let script = r#"
INSERT INTO "CBS"."DICT_ADM_ROUTE" ("ADMIN_NAME", "ENGLISH_NAME") VALUES ('门诊', 'OUTPATIENT');
INSERT INTO "CBS"."DICT_ADM_ROUTE" ("ADMIN_NAME") VALUES ('急诊');
"#;

    let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 2);
    assert_eq!(
        statements[0],
        "INSERT INTO `cbs`.`dict_adm_route` (`admin_name`, `english_name`) VALUES ('门诊', 'OUTPATIENT') ON DUPLICATE KEY UPDATE `admin_name` = VALUES(`admin_name`), `english_name` = VALUES(`english_name`)"
    );
    assert_eq!(
        statements[1],
        "INSERT INTO `cbs`.`dict_adm_route` (`admin_name`) VALUES ('急诊') ON DUPLICATE KEY UPDATE `admin_name` = VALUES(`admin_name`)"
    );
}

#[test]
fn prepares_postgres_sql_statements_from_oracle_style_script() {
    let script = r#"INSERT INTO "CBS"."DICT_ADM_ROUTE" ("ADMIN_NAME") VALUES ('门诊');"#;

    let statements = prepare_sql_statements(script, &DbType::PostgreSQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert_eq!(
        statements[0],
        "INSERT INTO \"cbs\".\"dict_adm_route\" (\"admin_name\") VALUES ('门诊')"
    );
}

#[test]
fn prepares_mysql_insert_select_with_duplicate_key_update() {
    let script =
        r#"INSERT INTO "CBS"."DICT_DOCTOR_TITLE" ("ID", "NAME") SELECT '1', '主任医师' FROM DUAL;"#;

    let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert_eq!(
        statements[0],
        "INSERT INTO `cbs`.`dict_doctor_title` (`id`, `name`) SELECT '1', '主任医师' FROM DUAL ON DUPLICATE KEY UPDATE `id` = VALUES(`id`), `name` = VALUES(`name`)"
    );
}

#[test]
fn prepares_mysql_insert_without_schema_using_target_db() {
    let script = r#"INSERT INTO "DICT_DRUG_CATE" ("ID", "NAME") VALUES ('1', '西药');"#;

    let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert_eq!(
        statements[0],
        "INSERT INTO `cbs`.`dict_drug_cate` (`id`, `name`) VALUES ('1', '西药') ON DUPLICATE KEY UPDATE `id` = VALUES(`id`), `name` = VALUES(`name`)"
    );
}

#[test]
fn prepares_mysql_truncate_without_schema_using_target_db() {
    let script = r#"TRUNCATE TABLE "DICT_DRUG_CATE";"#;

    let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert_eq!(statements[0], "TRUNCATE TABLE `cbs`.`dict_drug_cate`");
}

#[test]
fn prepares_mysql_truncate_with_schema_keeps_prefix() {
    let script = r#"TRUNCATE TABLE "CBS"."DICT_DRUG_CATE";"#;

    let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert_eq!(statements[0], "TRUNCATE TABLE `cbs`.`dict_drug_cate`");
}

#[test]
fn prepares_mysql_delete_without_schema_using_target_db() {
    let script = r#"DELETE FROM "DICT_DRUG_CATE";"#;

    let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert_eq!(statements[0], "DELETE FROM `cbs`.`dict_drug_cate`");
}

#[test]
fn prepares_mysql_delete_with_schema_keeps_prefix() {
    let script = r#"DELETE FROM "CBS"."DICT_DRUG_CATE" WHERE ID = '1';"#;

    let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert_eq!(
        statements[0],
        "DELETE FROM `cbs`.`dict_drug_cate` WHERE ID = '1'"
    );
}

#[test]
fn prepares_mysql_update_without_schema_using_target_db() {
    let script = r#"UPDATE "DICT_DRUG_CATE" SET NAME = 'test';"#;

    let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert_eq!(
        statements[0],
        "UPDATE `cbs`.`dict_drug_cate` SET NAME = 'test'"
    );
}

#[test]
fn preserves_non_dml_statements() {
    let script = r#"SELECT 1 FROM DUAL;"#;

    let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert_eq!(statements[0], "SELECT 1 FROM DUAL");
}

#[test]
fn handles_compact_values_format() {
    // 紧凑格式：)VALUES(，VALUES 前无空格
    let script = r#"INSERT INTO "CBS"."DICT_ADM_ROUTE" ("ADMIN_NAME")VALUES('膀胱冲洗用');"#;

    let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert_eq!(
        statements[0],
        "INSERT INTO `cbs`.`dict_adm_route` (`admin_name`)VALUES('膀胱冲洗用') ON DUPLICATE KEY UPDATE `admin_name` = VALUES(`admin_name`)"
    );
}

#[test]
fn handles_compact_values_format_with_trailing_space() {
    // 紧凑格式：)VALUES (，VALUES 后紧跟空格和左括号
    let script = r#"INSERT INTO "CBS"."DICT_ADM_ROUTE" ("ADMIN_NAME")VALUES ('膀胱冲洗用');"#;

    let statements = prepare_sql_statements(script, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert!(
        statements[0].contains("ON DUPLICATE KEY UPDATE"),
        "should add ON DUPLICATE KEY UPDATE for compact VALUES format"
    );
}

#[test]
fn find_sql_keyword_finds_with_space_before() {
    let upper = "INSERT INTO TBL (A) VALUES (1)";
    assert_eq!(find_sql_keyword(upper, "VALUES"), Some(20));
}

#[test]
fn find_sql_keyword_finds_with_paren_before() {
    let upper = "INSERT INTO TBL (A)VALUES (1)";
    assert_eq!(find_sql_keyword(upper, "VALUES"), Some(19));
}

#[test]
fn find_sql_keyword_finds_with_backtick_before() {
    let upper = "INSERT INTO `TBL` (`A`)VALUES (1)";
    assert_eq!(find_sql_keyword(upper, "VALUES"), Some(23));
}

#[test]
fn find_sql_keyword_ignores_values_in_identifier() {
    let upper = "INSERT INTO MY_VALUES_TABLE (A) VALUES (1)";
    let pos = find_sql_keyword(upper, "VALUES").unwrap();
    // 应该匹配第二个 VALUES (关键字)，而不是 MY_VALUES_TABLE 中的 VALUES
    assert!(pos > 20, "should match keyword VALUES, not identifier prefix");
    assert_eq!(pos, 32);
}

#[test]
fn converts_oracle_to_date_to_mysql_str_to_date() {
    let sql = r#"INSERT INTO "CBS"."DICT_DRUG_CATE" ("ID", "NAME", "CREATE_TIME", "MODIFY_TIME", "DC_STATUS") VALUES ('0801335e36094bad8fd22096a0ac6dc0', '生物制品-细胞因子', TO_DATE('2025-05-16 09:41:01', 'YYYY-MM-DD HH24:MI:SS'), TO_DATE('2025-05-16 09:41:01', 'YYYY-MM-DD HH24:MI:SS'), 1);"#;

    let statements = prepare_sql_statements(sql, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert!(
        statements[0].contains("STR_TO_DATE"),
        "should convert TO_DATE to STR_TO_DATE for MySQL, got: {}",
        statements[0]
    );
    assert!(
        statements[0].contains("%Y-%m-%d %H:%i:%s"),
        "should convert Oracle format to MySQL format, got: {}",
        statements[0]
    );
    // STR_TO_DATE 中也包含 TO_DATE( 子串，需排除
    let upper = statements[0].to_uppercase();
    let has_oracle_to_date = upper.match_indices("TO_DATE(").any(|(idx, _)| {
        idx < 4 || &upper[idx - 4..idx] != "STR_"
    });
    assert!(
        !has_oracle_to_date,
        "should not contain Oracle TO_DATE( after conversion, got: {}",
        statements[0]
    );
}

#[test]
fn converts_oracle_to_date_to_pg_to_timestamp() {
    let sql = r#"INSERT INTO "CBS"."DICT_DRUG_CATE" ("ID", "NAME", "CREATE_TIME") VALUES ('id1', 'test', TO_DATE('2025-05-16 09:41:01', 'YYYY-MM-DD HH24:MI:SS'));"#;

    let statements = prepare_sql_statements(sql, &DbType::PostgreSQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert!(
        statements[0].contains("TO_TIMESTAMP"),
        "should convert TO_DATE to TO_TIMESTAMP for PostgreSQL, got: {}",
        statements[0]
    );
    assert!(
        statements[0].contains("HH24:MI:SS"),
        "should keep Oracle-compatible format for PostgreSQL, got: {}",
        statements[0]
    );
    // TO_TIMESTAMP 中包含 TO_ 前缀，需检查不含独立 TO_DATE(
    let upper = statements[0].to_uppercase();
    assert!(
        !upper.contains("TO_DATE("),
        "should not contain Oracle TO_DATE( after conversion, got: {}",
        statements[0]
    );
}

#[test]
fn does_not_convert_to_date_for_oracle_target() {
    let sql = r#"INSERT INTO "CBS"."DICT_DRUG_CATE" ("CREATE_TIME") VALUES (TO_DATE('2025-05-16', 'YYYY-MM-DD'));"#;

    let statements = prepare_sql_statements(sql, &DbType::Oracle, "cbs", false);

    assert_eq!(statements.len(), 1);
    assert!(
        statements[0].contains("TO_DATE"),
        "should keep TO_DATE unchanged for Oracle target"
    );
}

#[test]
fn converts_multiple_to_date_in_single_statement() {
    let sql = r#"INSERT INTO TBL (A, B) VALUES (TO_DATE('2025-01-01', 'YYYY-MM-DD'), TO_DATE('2025-06-28', 'YYYY-MM-DD'));"#;

    let statements = prepare_sql_statements(sql, &DbType::MySQL, "cbs", false);

    assert_eq!(statements.len(), 1);
    let stmt = &statements[0];
    let count = stmt.match_indices("STR_TO_DATE").count();
    assert_eq!(count, 2, "should convert both TO_DATE calls, got: {}", stmt);
}

#[test]
fn filters_trailing_export_count_comment() {
    // 测试 Oracle 类型（不做 MySQL 特殊处理）
    let script = "INSERT INTO t VALUES (1);\nINSERT INTO t VALUES (2);\n-- 共导出 2 条记录";
    let statements = prepare_sql_statements(script, &DbType::Oracle, "test_db", false);
    assert_eq!(statements.len(), 2, "should filter out trailing comment line for Oracle");

    let statements = prepare_sql_statements(script, &DbType::PostgreSQL, "test_db", false);
    assert_eq!(statements.len(), 2, "should filter out trailing comment line for PostgreSQL");

    // MySQL truncate_first=true 时也应正确过滤
    let statements = prepare_sql_statements(script, &DbType::MySQL, "test_db", true);
    assert_eq!(statements.len(), 2, "should filter out trailing comment line for MySQL (truncate)");

    // MySQL truncate_first=false 时也应正确过滤（之前会 panic）
    let statements = prepare_sql_statements(script, &DbType::MySQL, "test_db", false);
    assert_eq!(statements.len(), 2, "should filter out trailing comment line for MySQL (no truncate)");
}
