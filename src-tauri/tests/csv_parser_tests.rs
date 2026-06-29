use base_import_tool_lib::csv_parser::scan_folder;
use std::fs;

#[test]
fn scans_csv_files_grouped_by_parent_database_folder() {
    let root = std::env::temp_dir().join(format!(
        "base-import-tool-csv-parser-{}",
        std::process::id()
    ));
    let kbe_dir = root.join("kbe");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&kbe_dir).unwrap();
    fs::write(kbe_dir.join("drug_list.csv"), "id,name\n1,药品\n").unwrap();

    let files = scan_folder(root.to_str().unwrap()).unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].target_db, "kbe");
    assert_eq!(files[0].table_name, "drug_list");
    assert_eq!(files[0].row_count, None);

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn ignores_prefixed_csv_files_outside_database_folders() {
    let root = std::env::temp_dir().join(format!(
        "base-import-tool-csv-parser-prefix-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("kbe_drug_list.csv"), "id,name\n1,药品\n").unwrap();

    let files = scan_folder(root.to_str().unwrap()).unwrap();

    assert!(files.is_empty());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn skips_export_progress_csv_files() {
    let root = std::env::temp_dir().join(format!(
        "base-import-tool-csv-parser-progress-{}",
        std::process::id()
    ));
    let db_dir = root.join("drug_spec");
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&db_dir).unwrap();
    fs::write(
        db_dir.join("datamanage_progress.csv"),
        "ID,THREADNAME\n1,thread-1\n",
    )
    .unwrap();
    fs::write(db_dir.join("attribute_dict.csv"), "id,name\n1,属性\n").unwrap();

    let files = scan_folder(root.to_str().unwrap()).unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].table_name, "attribute_dict");

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn scans_sql_files_named_with_target_db_prefix() {
    let root = std::env::temp_dir().join(format!(
        "base-import-tool-sql-parser-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(
        root.join("clin_wkst_cw_app_menu.sql"),
        "insert into clin_wkst.cw_app_menu(id) values (1);",
    )
    .unwrap();

    let files = scan_folder(root.to_str().unwrap()).unwrap();

    assert_eq!(files.len(), 1);
    assert_eq!(files[0].target_db, "clin_wkst");
    assert_eq!(files[0].table_name, "cw_app_menu");
    assert_eq!(files[0].columns, Vec::<String>::new());

    let _ = fs::remove_dir_all(&root);
}

#[test]
fn ignores_sql_files_without_known_target_db_prefix() {
    let root = std::env::temp_dir().join(format!(
        "base-import-tool-sql-parser-ignore-{}",
        std::process::id()
    ));
    let _ = fs::remove_dir_all(&root);
    fs::create_dir_all(&root).unwrap();
    fs::write(root.join("hospital_custom.sql"), "select 1;").unwrap();

    let files = scan_folder(root.to_str().unwrap()).unwrap();

    assert!(files.is_empty());

    let _ = fs::remove_dir_all(&root);
}
