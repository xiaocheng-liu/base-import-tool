export type DbType = 'Oracle' | 'DM' | 'PostgreSQL' | 'MySQL';
export type OracleConnectionMode = 'ServiceName' | 'SID';
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
  oracle_connection_mode?: OracleConnectionMode;
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
  /** 失败 SQL 列表（每条独立展示，含完整 SQL） */
  errors?: SqlErrorItem[];
}

export interface ImportProgress {
  task_id: string;
  status: ImportStatus;
  progress: number;
  total_rows: number;
  imported_rows: number;
  error_message: string | null;
  /** 失败 SQL 列表（每条独立展示，含完整 SQL） */
  errors?: SqlErrorItem[];
}

export interface SqlErrorItem {
  /** 第几条 SQL（从 1 开始） */
  index: number;
  /** 错误简述 */
  error: string;
  /** 完整出错的 SQL 语句 */
  sql: string;
  /** 解决建议（可选） */
  suggestion?: string | null;
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
