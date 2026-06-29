use base_import_tool_lib::db::mysql_conn::MySqlConnection;
use base_import_tool_lib::models::TableIdentifier;

#[test]
fn formats_mysql_table_name_with_schema() {
    let table = TableIdentifier {
        schema: "drug_spec".to_string(),
        table_name: "attribute_dict".to_string(),
    };

    assert_eq!(
        MySqlConnection::format_table_name(&table),
        "`drug_spec`.`attribute_dict`"
    );
}
