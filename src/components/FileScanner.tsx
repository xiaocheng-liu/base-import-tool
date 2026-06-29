import { useState } from 'react';
import type { CsvFileInfo, TableSchemaInfo } from '../types';

interface Props {
  csvFiles: CsvFileInfo[];
  setCsvFiles: (files: CsvFileInfo[]) => void;
  selectedFiles: Set<string>;
  setSelectedFiles: (files: Set<string>) => void;
  onFilesScanned?: () => void;
  onNext?: () => void;
  embedded?: boolean;
  schemaDir?: string; // 可选：DDL 文件所在目录，如 .../release_202603/02_schema
}

export default function FileScanner({
  csvFiles,
  setCsvFiles,
  selectedFiles,
  setSelectedFiles,
  onFilesScanned,
  onNext,
  embedded = false,
  schemaDir: propSchemaDir,
}: Props) {
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [folderPath, setFolderPath] = useState('');
  const [schemaInfo, setSchemaInfo] = useState<TableSchemaInfo | null>(null);
  const [schemaLoading, setSchemaLoading] = useState(false);

  const handleSelectFolder = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({
        directory: true,
        multiple: false,
        title: '选择包含 CSV 文件的文件夹',
      });

      if (selected && typeof selected === 'string') {
        setFolderPath(selected);
        await scanFolder(selected);
      }
    } catch (e: any) {
      setError(`选择文件夹失败: ${e}`);
    }
  };

  const scanFolder = async (path: string) => {
    setLoading(true);
    setError('');
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const files: CsvFileInfo[] = await invoke('scan_csv_files', {
        folderPath: path,
      });
      setCsvFiles(files);
      // 默认全选
      setSelectedFiles(new Set(files.map((f) => f.file_path)));
      onFilesScanned?.();
    } catch (e: any) {
      setError(`扫描失败: ${e}`);
    } finally {
      setLoading(false);
    }
  };

  const toggleFile = (filePath: string) => {
    const next = new Set(selectedFiles);
    if (next.has(filePath)) {
      next.delete(filePath);
    } else {
      next.add(filePath);
    }
    setSelectedFiles(next);
  };

  const handleClear = () => {
    setCsvFiles([]);
    setSelectedFiles(new Set());
    setFolderPath('');
    setError('');
    onFilesScanned?.();
  };

  const toggleAll = () => {
    if (selectedFiles.size === csvFiles.length) {
      setSelectedFiles(new Set());
    } else {
      setSelectedFiles(new Set(csvFiles.map((f) => f.file_path)));
    }
  };

  const handleViewSchema = async (targetDb: string, tableName: string) => {
    // CSV 文件：直接显示扫描到的字段
    const file = csvFiles.find(
      (f) => f.target_db === targetDb && f.table_name === tableName
    );
    if (file && file.file_type === 'Csv' && file.columns.length > 0) {
      setSchemaInfo({
        table_name: tableName,
        table_comment: null,
        columns: file.columns.map((name) => ({
          name,
          data_type: '',
          nullable: true,
          comment: null,
        })),
      });
      return;
    }

    // SQL 文件：从 DDL 文件解析
    let schemaDirPath = propSchemaDir;
    if (!schemaDirPath && file) {
      // 从 SQL 文件路径推断：找到 SQL 文件所在目录的父目录，再找 02_schema
      const sqlDir = file.file_path.substring(0, file.file_path.lastIndexOf('/') || file.file_path.lastIndexOf('\\'));
      const parentDir = sqlDir.replace(/[\\/]?$/, '').replace(/[\\/][^\\/]+$/, '');
      schemaDirPath = `${parentDir}/02_schema`;
    }
    if (!schemaDirPath) {
      setError('无法推断 Schema 目录，请手动选择 02_schema 文件夹');
      return;
    }
    setSchemaLoading(true);
    setSchemaInfo(null);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const info: TableSchemaInfo = await invoke('get_table_schema', {
        schemaDir: schemaDirPath,
        targetDb,
        tableName,
      });
      setSchemaInfo(info);
    } catch (e: any) {
      setSchemaInfo(null);
      setError(`获取表结构失败: ${e}`);
    } finally {
      setSchemaLoading(false);
    }
  };

  const closeSchema = () => {
    setSchemaInfo(null);
  };

  // 按数据库分组
  const grouped = csvFiles.reduce(
    (acc, f) => {
      const db = f.target_db;
      if (!acc[db]) acc[db] = [];
      acc[db].push(f);
      return acc;
    },
    {} as Record<string, CsvFileInfo[]>
  );

  const fileTypeLabel = (fileType: CsvFileInfo['file_type']) =>
    fileType === 'Sql' ? 'SQL' : 'CSV';

  return (
    <div className="file-scanner">
      {!embedded && (
        <div className="section-header">
          <h2>文件扫描</h2>
          <p>选择包含 CSV 或 SQL 文件的目录，程序自动识别待导入数据</p>
        </div>
      )}

      <div className="folder-selector">
        <button className="btn btn-primary" onClick={handleSelectFolder} disabled={loading}>
          📁 选择文件夹
        </button>
        {folderPath && <span className="folder-path" title={folderPath}>{folderPath}</span>}
      </div>

      {loading && (
        <div className="loading">
          <div className="spinner" />
          <span>正在扫描导入文件...</span>
        </div>
      )}

      {error && <div className="error-msg">{error}</div>}

      {csvFiles.length > 0 && (
        <>
          <div className="scan-summary">
            <span>共扫描到 {csvFiles.length} 个导入文件</span>
            <div className="scan-summary-actions">
              <button className="btn btn-sm" onClick={toggleAll}>
                {selectedFiles.size === csvFiles.length ? '取消全选' : '全选'}
              </button>
              <button className="btn btn-sm btn-danger" onClick={handleClear}>
                清空
              </button>
            </div>
          </div>

          <div className="db-groups">
            {Object.entries(grouped).map(([db, files]) => (
              <div key={db} className="db-group">
                <h3 className="db-group-title">
                  {db.toUpperCase()} ({files.length} 个表)
                </h3>
                <div className="file-grid">
                  {files.map((f) => (
                    <label
                      key={f.file_path}
                      className={`file-card ${selectedFiles.has(f.file_path) ? 'selected' : ''}`}
                    >
                      <input
                        type="checkbox"
                        checked={selectedFiles.has(f.file_path)}
                        onChange={() => toggleFile(f.file_path)}
                      />
                      <div className="file-info">
                        <div className="file-title-row">
                          <span
                            className="table-name table-name-clickable"
                            onClick={(e) => {
                              e.preventDefault();
                              handleViewSchema(f.target_db, f.table_name);
                            }}
                            title="点击查看表字段和注释"
                          >
                            {f.table_name}
                          </span>
                          <span className={`file-type-badge file-type-${f.file_type.toLowerCase()}`}>
                            {fileTypeLabel(f.file_type)}
                          </span>
                        </div>
                        <span className="file-meta">
                          {f.file_type === 'Sql'
                            ? f.file_name
                            : `${f.columns.length} 列${f.row_count !== null ? ` · ${f.row_count} 行` : ''}`}
                        </span>
                      </div>
                    </label>
                  ))}
                </div>
              </div>
            ))}
          </div>

          {onNext && (
            <div className="actions">
              <button className="btn btn-primary" onClick={onNext}>
                下一步：配置数据库 →
              </button>
            </div>
          )}
        </>
      )}

      {/* 表字段/注释弹窗 */}
      {(schemaInfo || schemaLoading) && (
        <div className="modal-overlay" onClick={closeSchema}>
          <div className="modal-content schema-modal" onClick={(e) => e.stopPropagation()}>
            {schemaLoading ? (
              <div className="loading">
                <div className="spinner" />
                <span>正在加载表结构...</span>
              </div>
            ) : schemaInfo ? (
              <>
                <div className="modal-header">
                  <h3>
                    {schemaInfo.table_name}
                    {schemaInfo.table_comment && (
                      <span className="table-comment-label"> — {schemaInfo.table_comment}</span>
                    )}
                  </h3>
                  <button className="btn-close" onClick={closeSchema}>✕</button>
                </div>
                <div className="schema-columns-table-wrap">
                  <table className="schema-columns-table">
                    <thead>
                      <tr>
                        <th>字段名</th>
                        <th>类型</th>
                        <th>非空</th>
                        <th>注释</th>
                      </tr>
                    </thead>
                    <tbody>
                      {schemaInfo.columns.map((col, i) => (
                        <tr key={i}>
                          <td className="col-name">{col.name}</td>
                          <td className="col-type">{col.data_type}</td>
                          <td className="col-nullable">
                            {col.nullable ? '' : '✓'}
                          </td>
                          <td className="col-comment">{col.comment || '—'}</td>
                        </tr>
                      ))}
                    </tbody>
                  </table>
                </div>
                <div className="modal-footer">
                  <span className="col-count">共 {schemaInfo.columns.length} 个字段</span>
                  <button className="btn btn-sm" onClick={closeSchema}>关闭</button>
                </div>
              </>
            ) : null}
          </div>
        </div>
      )}
    </div>
  );
}
