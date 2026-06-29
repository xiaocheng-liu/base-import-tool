export type DbType = 'Oracle' | 'DM' | 'PostgreSQL' | 'MySQL';
export type ImportFileType = 'Csv' | 'Sql';
export type TargetDb =
  | 'cbs'
  | 'clin_wkst'
  | 'drug_spec'
  | 'his'
  | 'inpt'
  | 'kb_docs'
  | 'kbe'
  | 'outpt'
  | 'procure';
export type ImportStatus = 'Pending' | 'Running' | 'Completed' | 'Failed' | 'Skipped';

export interface CsvFileInfo {
  file_name: string;
  file_path: string;
  file_type: ImportFileType;
  target_db: string;
  table_name: string;
  row_count: number | null;
  columns: string[];
}

export interface DbConfig {
  id: string;
  db_type: DbType;
  target_db?: TargetDb;
  host: string;
  port: number;
  username: string;
  password: string;
  database: string;
  extra_params: string;
}

export interface ImportTask {
  id: string;
  csv_file: CsvFileInfo;
  db_config_id: string;
  status: ImportStatus;
  progress: number;
  total_rows: number;
  imported_rows: number;
  error_message: string | null;
}

export interface ImportProgress {
  task_id: string;
  status: ImportStatus;
  progress: number;
  total_rows: number;
  imported_rows: number;
  error_message: string | null;
}

export interface ConnectionTestResult {
  success: boolean;
  message: string;
  db_version: string | null;
}

export interface SchemaTarget {
  target_db: string;
  tables_file: string;
  indexes_file: string;
}

export interface ColumnWithComment {
  name: string;
  data_type: string;
  nullable: boolean;
  comment: string | null;
}

export interface TableSchemaInfo {
  table_name: string;
  table_comment: string | null;
  columns: ColumnWithComment[];
}
