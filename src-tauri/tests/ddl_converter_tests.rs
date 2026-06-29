use base_import_tool_lib::ddl_converter::{should_expand_column, ColumnDef, DdlConverter, IndexDef, TableDef};
use base_import_tool_lib::models::{ColumnInfo, DbType};
use std::collections::HashMap;
use std::fs;

#[test]
fn lists_schema_targets_from_tables_and_indexes_files() {
    let root = std::env::temp_dir().join(format!(
        "base-import-tool-schema-targets-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("drug_spec_tables.sql"), "").unwrap();
    fs::write(root.join("drug_spec_indexes.sql"), "").unwrap();
    fs::write(root.join("kbe_tables.sql"), "").unwrap();
    fs::write(root.join("orphan_tables.sql"), "").unwrap();

    let converter = DdlConverter::new(root.clone());
    let targets = converter.list_schema_targets().unwrap();

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].target_db, "drug_spec");
    assert_eq!(targets[0].tables_file, "drug_spec_tables.sql");
    assert_eq!(targets[0].indexes_file, "drug_spec_indexes.sql");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn lists_unknown_schema_targets_from_file_pairs() {
    let root = std::env::temp_dir().join(format!(
        "base-import-tool-any-schema-targets-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("hospital_custom_tables.sql"), "").unwrap();
    fs::write(root.join("hospital_custom_indexes.sql"), "").unwrap();

    let converter = DdlConverter::new(root.clone());
    let targets = converter.list_schema_targets().unwrap();

    assert_eq!(targets.len(), 1);
    assert_eq!(targets[0].target_db, "hospital_custom");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn detects_text_column_expansion() {
    let existing = ColumnInfo {
        name: "NAME".to_string(),
        data_type: "VARCHAR2".to_string(),
        data_length: Some(64),
        data_precision: None,
        data_scale: None,
    };
    let expected = ColumnDef {
        name: "NAME".to_string(),
        data_type: "VARCHAR2(128)".to_string(),
        nullable: true,
        default_value: None,
    };

    assert!(should_expand_column(&existing, &expected));
}

#[test]
fn does_not_expand_mysql_text_for_large_oracle_varchar() {
    let existing = ColumnInfo {
        name: "HIS_NAME".to_string(),
        data_type: "text".to_string(),
        data_length: None,
        data_precision: None,
        data_scale: None,
    };
    let expected = ColumnDef {
        name: "HIS_NAME".to_string(),
        data_type: "VARCHAR2(1000)".to_string(),
        nullable: true,
        default_value: None,
    };

    assert!(!should_expand_column(&existing, &expected));
}

#[test]
fn expands_mysql_varchar_to_text_for_large_oracle_varchar() {
    let existing = ColumnInfo {
        name: "HIS_NAME".to_string(),
        data_type: "varchar".to_string(),
        data_length: Some(255),
        data_precision: None,
        data_scale: None,
    };
    let expected = ColumnDef {
        name: "HIS_NAME".to_string(),
        data_type: "VARCHAR2(1000)".to_string(),
        nullable: true,
        default_value: None,
    };

    assert!(should_expand_column(&existing, &expected));
}

#[test]
fn does_not_expand_mysql_int_for_same_mapped_oracle_number() {
    let existing = ColumnInfo {
        name: "SORT_NO".to_string(),
        data_type: "int".to_string(),
        data_length: None,
        data_precision: Some(10),
        data_scale: Some(0),
    };
    let expected = ColumnDef {
        name: "SORT_NO".to_string(),
        data_type: "NUMBER(9,0)".to_string(),
        nullable: true,
        default_value: None,
    };

    assert!(!should_expand_column(&existing, &expected));
}

#[test]
fn does_not_expand_for_oracle_number_with_wildcard_precision() {
    let existing = ColumnInfo {
        name: "DATA_SCOPE".to_string(),
        data_type: "bigint".to_string(),
        data_length: None,
        data_precision: Some(19),
        data_scale: Some(0),
    };
    let expected = ColumnDef {
        name: "DATA_SCOPE".to_string(),
        data_type: "NUMBER(*,0)".to_string(),
        nullable: true,
        default_value: None,
    };

    assert!(!should_expand_column(&existing, &expected));
}

#[test]
fn does_not_expand_period_time_for_wildcard_number_precision() {
    let existing = ColumnInfo {
        name: "PERIOD_TIME".to_string(),
        data_type: "bigint".to_string(),
        data_length: None,
        data_precision: Some(19),
        data_scale: Some(0),
    };
    let expected = ColumnDef {
        name: "PERIOD_TIME".to_string(),
        data_type: "NUMBER(*,0)".to_string(),
        nullable: true,
        default_value: None,
    };

    assert!(!should_expand_column(&existing, &expected));
}

#[test]
fn maps_large_oracle_varchar_to_mysql_text() {
    assert_eq!(
        DdlConverter::map_oracle_type("VARCHAR2(1000)", &DbType::MySQL),
        "TEXT"
    );
    assert_eq!(
        DdlConverter::map_oracle_type("VARCHAR2(4000)", &DbType::MySQL),
        "TEXT"
    );
}

#[test]
fn maps_long_oracle_char_to_mysql_varchar() {
    assert_eq!(
        DdlConverter::map_oracle_type("CHAR(64)", &DbType::MySQL),
        "CHAR(64)"
    );
    assert_eq!(
        DdlConverter::map_oracle_type("CHAR(500)", &DbType::MySQL),
        "VARCHAR(500)"
    );
}

#[test]
fn maps_oracle_timestamp_with_precision_to_mysql_datetime() {
    assert_eq!(
        DdlConverter::map_oracle_type("TIMESTAMP (6)", &DbType::MySQL),
        "DATETIME"
    );
}

#[test]
fn parses_oracle_index_table_name() {
    let converter = DdlConverter::new(std::env::temp_dir());
    let indexes = converter.parse_indexes(
        r#"CREATE UNIQUE INDEX "CBS"."DICT_ADM_ROUTE_MAP_UQ" ON "CBS"."DICT_ADM_ROUTE_MAP" ("HIS_NAME") ;"#,
    );

    assert_eq!(indexes.len(), 1);
    assert_eq!(indexes[0].schema, "CBS");
    assert_eq!(indexes[0].index_name, "DICT_ADM_ROUTE_MAP_UQ");
    assert_eq!(indexes[0].table_name, "DICT_ADM_ROUTE_MAP");
    assert_eq!(indexes[0].columns, vec!["HIS_NAME"]);
}

#[test]
fn parses_oracle_expression_index_as_unsupported_column() {
    let converter = DdlConverter::new(std::env::temp_dir());
    let indexes = converter.parse_indexes(
        r#"CREATE UNIQUE INDEX "CBS"."IDX_DICT_TPN_DRUG_UNQ" ON "CBS"."DICT_TPN_DRUG" ("DRUG_CODE", "HOSPITAL_ID", NVL("DRUG_NAME",'-')) ;"#,
    );

    assert_eq!(indexes.len(), 1);
    assert_eq!(
        indexes[0].columns,
        vec![
            "DRUG_CODE".to_string(),
            "HOSPITAL_ID".to_string(),
            "NVL(\"DRUG_NAME\",'-')".to_string()
        ]
    );
    assert!(!indexes[0].is_plain_column_index());
}

#[test]
fn adds_mysql_prefix_length_for_text_index_columns() {
    let table = TableDef {
        schema: "CBS".to_string(),
        table_name: "DICT_ADM_ROUTE_MAP".to_string(),
        columns: vec![ColumnDef {
            name: "HIS_NAME".to_string(),
            data_type: "VARCHAR2(1000)".to_string(),
            nullable: true,
            default_value: None,
        }],
        primary_key: Vec::new(),
        comment: None,
        column_comments: HashMap::new(),
    };
    let index = IndexDef {
        schema: "CBS".to_string(),
        table_name: "DICT_ADM_ROUTE_MAP".to_string(),
        index_name: "DICT_ADM_ROUTE_MAP_UQ".to_string(),
        unique: true,
        columns: vec!["HIS_NAME".to_string()],
    };

    let ddl = DdlConverter::generate_create_index_for_table(
        &index,
        Some(&table),
        &DbType::MySQL,
    )
    .unwrap();

    assert_eq!(
        ddl,
        "CREATE UNIQUE INDEX `dict_adm_route_map_uq` ON `cbs`.`dict_adm_route_map` (`his_name`(191));"
    );
}

#[test]
fn generates_mysql_table_in_schema_database() {
    let table = TableDef {
        schema: "DRUG_SPEC".to_string(),
        table_name: "ATTRIBUTE_DICT".to_string(),
        columns: vec![ColumnDef {
            name: "ID".to_string(),
            data_type: "VARCHAR2(64)".to_string(),
            nullable: false,
            default_value: None,
        }],
        primary_key: vec!["ID".to_string()],
        comment: None,
        column_comments: HashMap::new(),
    };

    let ddl = DdlConverter::generate_create_table(&table, &DbType::MySQL);

    assert!(ddl.starts_with("CREATE TABLE IF NOT EXISTS `drug_spec`.`attribute_dict`"));
}

#[test]
fn does_not_attach_later_alter_primary_key_to_previous_table() {
    let converter = DdlConverter::new(std::env::temp_dir());
    let tables = converter
        .parse_tables(
            r#"
CREATE TABLE "HIS"."ADM_ROUTE_DICT"
( "ADMINISTRATION_CODE" VARCHAR2(32) NOT NULL ENABLE
) ;

CREATE TABLE "HIS"."INP_EXAMINE"
( "ID" VARCHAR2(64) NOT NULL ENABLE
) ;
ALTER TABLE "HIS"."INP_EXAMINE" ADD PRIMARY KEY ("ID") ;
"#,
        )
        .unwrap();

    assert_eq!(tables.len(), 2);
    assert!(tables[0].primary_key.is_empty());
    assert_eq!(tables[1].primary_key, vec!["ID"]);
}

#[test]
fn formats_mysql_defaults_for_oracle_specific_values() {
    assert_eq!(
        DdlConverter::format_default_value(
            "DATETIME",
            &Some("SYSDATE".to_string()),
            &DbType::MySQL,
        ),
        " DEFAULT CURRENT_TIMESTAMP"
    );
    assert_eq!(
        DdlConverter::format_default_value(
            "BIGINT",
            &Some("\"CBS\".\"ID_SEQUENCE\".\"NEXTVAL\"".to_string()),
            &DbType::MySQL,
        ),
        ""
    );
    assert_eq!(
        DdlConverter::format_default_value(
            "TEXT",
            &Some("''".to_string()),
            &DbType::MySQL,
        ),
        ""
    );
    assert_eq!(
        DdlConverter::format_default_value(
            "TINYINT",
            &Some("''".to_string()),
            &DbType::MySQL,
        ),
        ""
    );
    assert_eq!(
        DdlConverter::format_default_value(
            "DATETIME",
            &Some("SYSTIMESTAMP".to_string()),
            &DbType::MySQL,
        ),
        " DEFAULT CURRENT_TIMESTAMP"
    );
    assert_eq!(
        DdlConverter::format_default_value(
            "DATETIME",
            &Some("''".to_string()),
            &DbType::MySQL,
        ),
        ""
    );
}

#[test]
fn generates_mysql_table_without_invalid_oracle_defaults() {
    let converter = DdlConverter::new(std::env::temp_dir());
    let tables = converter
        .parse_tables(
            r#"
CREATE TABLE "CBS"."SYS_MENU"
( "MENU_ID" NUMBER(20,0) DEFAULT "CBS"."ID_SEQUENCE"."NEXTVAL" NOT NULL ENABLE,
  "CONTENT" VARCHAR2(4000) DEFAULT '' NOT NULL ENABLE,
  "CREATE_TIME" DATE DEFAULT SYSDATE,
  "RULE_TYPE" NUMBER(1,0) DEFAULT '' NOT NULL ENABLE,
  CONSTRAINT "SYS_MENU_PK" PRIMARY KEY ("MENU_ID")
) ;
"#,
        )
        .unwrap();
    let ddl = DdlConverter::generate_create_table(&tables[0], &DbType::MySQL);

    assert!(ddl.contains("`menu_id` DECIMAL(20) NOT NULL"));
    assert!(ddl.contains("`content` TEXT NOT NULL"));
    assert!(ddl.contains("`create_time` DATETIME DEFAULT CURRENT_TIMESTAMP"));
    assert!(ddl.contains("`rule_type` TINYINT NOT NULL"));
    assert!(!ddl.contains("ID_SEQUENCE"));
    assert!(!ddl.contains("TEXT NOT NULL DEFAULT"));
    assert!(!ddl.contains("TINYINT NOT NULL DEFAULT ''"));
}

#[test]
fn generates_mysql_table_without_invalid_timestamp_defaults() {
    let converter = DdlConverter::new(std::env::temp_dir());
    let tables = converter
        .parse_tables(
            r#"
CREATE TABLE "INPT"."CHECK_PRES_INPUT_PRE"
( "PRES_UNIQUE_ID" VARCHAR2(100) NOT NULL ENABLE,
  "CREATE_TIME" TIMESTAMP (6) DEFAULT '',
  CONSTRAINT "SYS_C009704" PRIMARY KEY ("PRES_UNIQUE_ID")
) ;

CREATE TABLE "INPT"."MEDICATION_REASON"
( "ID" VARCHAR2(64) NOT NULL ENABLE,
  "CREATE_TIME" TIMESTAMP (6) DEFAULT SYSTIMESTAMP,
  CONSTRAINT "PK_MEDICATION_REASON" PRIMARY KEY ("ID")
) ;
"#,
        )
        .unwrap();

    let first_ddl =
        DdlConverter::generate_create_table(&tables[0], &DbType::MySQL);
    let second_ddl =
        DdlConverter::generate_create_table(&tables[1], &DbType::MySQL);

    assert!(first_ddl.contains("`create_time` DATETIME"));
    assert!(!first_ddl.contains("DEFAULT ''"));
    assert!(second_ddl.contains("`create_time` DATETIME DEFAULT CURRENT_TIMESTAMP"));
    assert!(!second_ddl.contains("'SYSTIMESTAMP'"));
}
