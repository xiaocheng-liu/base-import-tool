import { useEffect, useMemo, useRef, useState } from 'react';
import type {
  DbConfig,
  SchemaChangeKind,
  SchemaDiffReport,
  SchemaInitProgress,
  SchemaTarget,
} from '../types';

interface Props {
  dbConfigs: DbConfig[];
  selectedDbConfigId: string;
  setSelectedDbConfigId: (id: string) => void;
}

/** 解析日志行所属的库 */
function detectDbFromLogLine(line: string, dbNames: string[]): string | null {
  // 1. 显式库分隔线："-- 初始化库: cbs --"
  const sepMatch = line.match(/^--\s*初始化库:\s*(\w+)\s*--/);
  if (sepMatch) return sepMatch[1].toLowerCase();
  // 2. 失败/完成前缀："✗ 库 cbs 初始化失败: ..."
  const libMatch = line.match(/^[✗×] 库 (\w+) /);
  if (libMatch) return libMatch[1].toLowerCase();
  // 3. 表/索引操作："  ▶ 表 CBS.xxx" / "  ▶ 创建索引 IDX ..."
  const tableMatch = line.match(/(?:表|索引)\s+([A-Z_][A-Z0-9_]*)\./i);
  if (tableMatch) {
    const schema = tableMatch[1].toLowerCase();
    // schema 名通常就是库名，或可通过 dbNames 反查
    if (dbNames.includes(schema)) return schema;
  }
  return null;
}

function timestampLabel() {
  const now = new Date();
  return `${now.getFullYear()}${String(now.getMonth() + 1).padStart(2, '0')}${String(now.getDate()).padStart(2, '0')}_${String(now.getHours()).padStart(2, '0')}${String(now.getMinutes()).padStart(2, '0')}${String(now.getSeconds()).padStart(2, '0')}`;
}

export default function SchemaInit({ dbConfigs, selectedDbConfigId }: Props) {
  const [schemaTargets, setSchemaTargets] = useState<SchemaTarget[]>([]);
  const [schemaFolderPath, setSchemaFolderPath] = useState('');
  const [loading, setLoading] = useState(false);
  const [scanning, setScanning] = useState(false);
  const [diffReport, setDiffReport] = useState<SchemaDiffReport | null>(null);
  const [expandedDiffDbs, setExpandedDiffDbs] = useState<Record<string, boolean>>({});
  const [expandedDiffTables, setExpandedDiffTables] = useState<Record<string, boolean>>({});
  const [initializing, setInitializing] = useState(false);
  const [loadError, setLoadError] = useState('');
  const [globalLogs, setGlobalLogs] = useState<string[]>([]);
  const [dbLogs, setDbLogs] = useState<Record<string, string[]>>({});
  const [progressMap, setProgressMap] = useState<Record<string, SchemaInitProgress>>({});
  const [expandedDbs, setExpandedDbs] = useState<Record<string, boolean>>({});
  const pollingRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const taskIdsRef = useRef<string[]>([]);
  const logEndRefs = useRef<Record<string, HTMLDivElement | null>>({});
  const logContainerRefs = useRef<Record<string, HTMLDivElement | null>>({});
  const userScrolledUpRefs = useRef<Record<string, boolean>>({});

  const dbNames = useMemo(() => schemaTargets.map((t) => t.target_db.toLowerCase()), [schemaTargets]);

  const selectedConfig = dbConfigs.find((c) => c.id === selectedDbConfigId);

  useEffect(() => {
    setDiffReport(null);
    setExpandedDiffDbs({});
    setExpandedDiffTables({});
  }, [selectedDbConfigId]);

  useEffect(() => {
    if (!schemaFolderPath) return;

    const loadSchemaTargets = async () => {
      setLoading(true);
      setLoadError('');

      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const targets: SchemaTarget[] = await invoke('list_schema_targets', {
          schemaDir: schemaFolderPath,
        });
        setSchemaTargets(targets);
        setDiffReport(null);
      } catch (e: any) {
        setLoadError(`读取初始化脚本失败: ${e}`);
        setSchemaTargets([]);
      } finally {
        setLoading(false);
      }
    };

    loadSchemaTargets();
  }, [schemaFolderPath]);

  const handleSelectSchemaFolder = async () => {
    try {
      const { open } = await import('@tauri-apps/plugin-dialog');
      const selected = await open({
        directory: true,
        multiple: false,
        title: '选择包含 *_tables.sql 和 *_indexes.sql 的 Schema 文件夹',
      });

      if (selected && typeof selected === 'string') {
        setSchemaFolderPath(selected);
        setGlobalLogs([]);
        setDbLogs({});
        setExpandedDbs({});
        setDiffReport(null);
        setExpandedDiffDbs({});
        setExpandedDiffTables({});
      }
    } catch (e: any) {
      setLoadError(`选择 Schema 文件夹失败: ${e}`);
    }
  };

  const handleClear = () => {
    if (pollingRef.current) {
      clearInterval(pollingRef.current);
      pollingRef.current = null;
    }
    setSchemaTargets([]);
    setSchemaFolderPath('');
    setLoadError('');
    setGlobalLogs([]);
    setDbLogs({});
    setProgressMap({});
    setExpandedDbs({});
    setDiffReport(null);
    setExpandedDiffDbs({});
    setExpandedDiffTables({});
    setScanning(false);
    setInitializing(false);
  };

  const handleScanDiffs = async () => {
    if (!selectedConfig || !schemaFolderPath || schemaTargets.length === 0) return;
    setScanning(true);
    setLoadError('');
    setDiffReport(null);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const report: SchemaDiffReport = await invoke('scan_schema_diffs', {
        dbConfigId: selectedConfig.id,
        schemaDir: schemaFolderPath,
      });
      setDiffReport(report);
      const expanded: Record<string, boolean> = {};
      for (const database of report.databases) {
        expanded[database.target_db.toLowerCase()] =
          database.error !== null || database.tables.length > 0 || database.warnings.length > 0;
      }
      setExpandedDiffDbs(expanded);
    } catch (e: any) {
      setLoadError(`扫描结构差异失败: ${e}`);
    } finally {
      setScanning(false);
    }
  };

  const appendDbLogs = (lines: string[]) => {
    setDbLogs((prev) => {
      const next: Record<string, string[]> = { ...prev };
      let currentDb: string | null = null;
      for (const line of lines) {
        const detected = detectDbFromLogLine(line, dbNames);
        if (detected) currentDb = detected;
        if (currentDb) {
          const arr = next[currentDb] ? [...next[currentDb]!] : [];
          arr.push(line);
          next[currentDb] = arr;
        }
      }
      return next;
    });
  };

  const handleInit = async () => {
    if (!selectedConfig || schemaTargets.length === 0) return;
    setInitializing(true);
    setGlobalLogs([]);
    setDbLogs({});
    setProgressMap({});
    userScrolledUpRefs.current = {};
    // 默认全部展开，方便实时查看
    const expandAll: Record<string, boolean> = {};
    for (const t of schemaTargets) expandAll[t.target_db.toLowerCase()] = true;
    setExpandedDbs(expandAll);

    const logBuffer: string[] = [];
    let flushTimer: ReturnType<typeof setInterval> | null = null;

    const { listen } = await import('@tauri-apps/api/event');
    const unlistenLog = await listen<string>('schema-log', (event) => {
      const lines = event.payload.split('\n').filter((line) => line.length > 0);
      logBuffer.push(...lines);
      if (!flushTimer) {
        flushTimer = setInterval(() => {
          if (logBuffer.length > 0) {
            const batch = logBuffer.splice(0);
            setGlobalLogs((prev) => [...prev, ...batch]);
            appendDbLogs(batch);
          }
        }, 100);
      }
    });

    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const taskIds: string[] = await invoke('init_all_schemas', {
        dbConfigId: selectedConfig.id,
        schemaDir: schemaFolderPath,
      });
      taskIdsRef.current = taskIds;

      const pollProgress = async () => {
        const currentIds = taskIdsRef.current;
        if (currentIds.length === 0) return;
        try {
          const { invoke } = await import('@tauri-apps/api/core');
          const progress: Record<string, SchemaInitProgress> = await invoke(
            'get_schema_init_progress',
            { targetDbs: currentIds }
          );
          setProgressMap(progress);

          const allDone = currentIds.every((id) => {
            const p = progress[id];
            return p && (p.status === 'Completed' || p.status === 'Failed');
          });
          if (allDone) {
            if (pollingRef.current) {
              clearInterval(pollingRef.current);
              pollingRef.current = null;
            }
            setTimeout(() => {
              if (flushTimer) clearInterval(flushTimer);
              if (logBuffer.length > 0) {
                const batch = logBuffer.splice(0);
                setGlobalLogs((prev) => [...prev, ...batch]);
                appendDbLogs(batch);
              }
              unlistenLog();
              setInitializing(false);
              setDiffReport(null);
            }, 300);
          }
        } catch {
          // 轮询失败静默处理
        }
      };

      await pollProgress();
      pollingRef.current = setInterval(pollProgress, 1000);
    } catch (e: any) {
      if (flushTimer) clearInterval(flushTimer);
      if (logBuffer.length > 0) {
        const batch = logBuffer.splice(0);
        setGlobalLogs((prev) => [...prev, ...batch]);
        appendDbLogs(batch);
      }
      unlistenLog();
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
        pollingRef.current = null;
      }
      setGlobalLogs((prev) => [...prev, `✗ 初始化失败: ${e}`]);
      setInitializing(false);
    }
  };

  const toggleDb = (db: string) => {
    setExpandedDbs((prev) => ({ ...prev, [db]: !prev[db] }));
  };

  const changeLabel = (kind: SchemaChangeKind) => {
    const labels: Record<SchemaChangeKind, string> = {
      CreateTable: '新增表',
      AddColumn: '新增字段',
      ExpandColumn: '扩容字段',
      UpdateTableComment: '表注释',
      UpdateColumnComment: '字段注释',
    };
    return labels[kind];
  };

  const copyDbLogs = (db: string) => {
    const text = (dbLogs[db] || []).join('\n');
    navigator.clipboard.writeText(text).catch(() => {});
  };

  const exportDbLogs = (db: string) => {
    const text = (dbLogs[db] || []).join('\n');
    const blob = new Blob([text], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `schema_init_${db}_${timestampLabel()}.txt`;
    a.click();
    URL.revokeObjectURL(url);
  };

  const exportAllLogs = () => {
    const text = globalLogs.join('\n');
    const blob = new Blob([text], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = `schema_init_all_${timestampLabel()}.txt`;
    a.click();
    URL.revokeObjectURL(url);
  };

  const statusLabel = (status: string) => {
    switch (status) {
      case 'Running':
        return '初始化中';
      case 'Completed':
        return '已完成';
      case 'Failed':
        return '失败';
      default:
        return '等待中';
    }
  };

  const statusIcon = (status: string) => {
    switch (status) {
      case 'Completed':
        return '✅';
      case 'Failed':
        return '❌';
      case 'Running':
        return '🔄';
      default:
        return '⏳';
    }
  };

  const logLineClass = (line: string) => {
    if (line.startsWith('✗') || line.startsWith('×')) return 'log-error';
    if (line.startsWith('✓')) return 'log-success';
    if (line.startsWith('▶')) return 'log-header';
    if (line.startsWith('═')) return 'log-footer';
    if (line.startsWith('初始化完成') || line.startsWith('-- 初始化库:')) return 'log-summary';
    if (line.startsWith('    SQL:')) return 'log-sql';
    return '';
  };

  if (!selectedConfig) {
    return (
      <div className="schema-init">
        <div className="section-header">
          <h2>数据库初始化</h2>
          <p>选择 Schema 脚本目录，一键初始化所有库的表结构</p>
        </div>
        <div className="empty-state">
          <p>请先在「数据库连接配置」中保存连接配置</p>
        </div>
      </div>
    );
  }

  return (
    <div className="schema-init">
      <div className="section-header">
        <h2>数据库初始化</h2>
        <p>选择 Schema 脚本目录，一键初始化所有库的表结构</p>
      </div>

      {loadError && <div className="error-msg">{loadError}</div>}

      <div className="import-summary">
        <div className="summary-card">
          <span className="summary-label">连接类型</span>
          <span className="summary-value">{selectedConfig.db_type}</span>
        </div>
        <div className="summary-card">
          <span className="summary-label">连接地址</span>
          <span className="summary-value">
            {selectedConfig.host}:{selectedConfig.port}
          </span>
        </div>
        {schemaTargets.length > 0 && (
          <div className="summary-card">
            <span className="summary-label">待初始化库</span>
            <span className="summary-value">{schemaTargets.length} 个</span>
          </div>
        )}
      </div>

      <div className="folder-selector">
        <button className="btn btn-primary" onClick={handleSelectSchemaFolder} disabled={loading}>
          📁 选择 Schema 文件夹
        </button>
        {schemaFolderPath && (
          <>
            <span className="folder-path" title={schemaFolderPath}>
              {schemaFolderPath}
            </span>
            <button className="btn btn-sm btn-danger" onClick={handleClear}>
              清空
            </button>
          </>
        )}
        {schemaFolderPath && schemaTargets.length > 0 && (
          <button
            className="btn btn-secondary"
            onClick={handleScanDiffs}
            disabled={loading || scanning || initializing}
          >
            {scanning ? '扫描中...' : '扫描结构差异'}
          </button>
        )}
      </div>

      {diffReport && (
        <div className="schema-diff-section">
          <div className="schema-diff-summary">
            <span><strong>{diffReport.database_count}</strong> 个库</span>
            <span><strong>{diffReport.new_table_count}</strong> 张新表</span>
            <span><strong>{diffReport.field_change_count}</strong> 个字段</span>
            <span><strong>{diffReport.comment_change_count}</strong> 个注释</span>
          </div>
          <div className="schema-diff-list">
            {diffReport.databases.map((database) => {
              const dbKey = database.target_db.toLowerCase();
              const expanded = !!expandedDiffDbs[dbKey];
              const consistent =
                !database.error && database.tables.length === 0 && database.warnings.length === 0;
              return (
                <div
                  key={dbKey}
                  className={`schema-diff-db ${database.error ? 'schema-diff-error' : consistent ? 'schema-diff-consistent' : ''}`}
                >
                  <button
                    type="button"
                    className="schema-diff-db-header"
                    onClick={() => setExpandedDiffDbs((prev) => ({ ...prev, [dbKey]: !prev[dbKey] }))}
                  >
                    <span className="schema-module-toggle">{expanded ? '▼' : '▶'}</span>
                    <strong>{database.target_db.toUpperCase()}</strong>
                    <span className="schema-diff-status">
                      {database.error
                        ? '扫描失败'
                        : consistent
                          ? '结构一致'
                          : database.executable_change_count > 0
                            ? `${database.executable_change_count} 项变更`
                            : '无可执行变更'}
                    </span>
                  </button>
                  {expanded && (
                    <div className="schema-diff-db-body">
                      {database.error && <div className="schema-db-error">{database.error}</div>}
                      {database.warnings.map((warning) => (
                        <div key={warning} className="schema-diff-warning">{warning}</div>
                      ))}
                      {!database.error && database.tables.length === 0 && (
                        <div className="schema-diff-empty">数据库结构与脚本一致</div>
                      )}
                      {database.tables.map((table) => {
                        const tableKey = `${dbKey}.${table.schema}.${table.table_name}`;
                        const tableExpanded = !!expandedDiffTables[tableKey];
                        return (
                          <div key={tableKey} className="schema-diff-table">
                            <button
                              type="button"
                              className="schema-diff-table-header"
                              onClick={() => setExpandedDiffTables((prev) => ({
                                ...prev,
                                [tableKey]: !prev[tableKey],
                              }))}
                            >
                              <span>{tableExpanded ? '▼' : '▶'}</span>
                              <strong>{table.schema}.{table.table_name}</strong>
                              <span>{table.is_new_table ? '新增表' : `${table.changes.length} 项`}</span>
                            </button>
                            {tableExpanded && (
                              <div className="schema-diff-items">
                                {table.is_new_table && (
                                  <>
                                    <div className="schema-diff-detail">字段：{table.new_table_columns.join(', ') || '无'}</div>
                                  </>
                                )}
                                {table.changes.map((change, index) => (
                                  <div key={`${change.kind}-${change.object_name}-${index}`} className="schema-diff-item">
                                    <span className={`schema-diff-kind schema-diff-kind-${change.kind.toLowerCase()}`}>
                                      {changeLabel(change.kind)}
                                    </span>
                                    <strong>{change.object_name}</strong>
                                    <span className="schema-diff-values">
                                      {change.current ?? '(无)'} → {change.target ?? '(删除)'}
                                    </span>
                                  </div>
                                ))}
                              </div>
                            )}
                          </div>
                        );
                      })}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}

      {schemaTargets.length > 0 &&
        (initializing || globalLogs.length > 0 || Object.keys(progressMap).length > 0) && (
        <div className="schema-module-list">
          {schemaTargets.map((target) => {
            const dbKey = target.target_db.toLowerCase();
            const p = progressMap[dbKey];
            const status = p?.status || 'Pending';
            const progressVal = p?.progress || 0;
            const totalTables = p?.total_tables || 0;
            const completedTables = p?.completed_tables || 0;
            const expanded = !!expandedDbs[dbKey];
            const logs = dbLogs[dbKey] || [];

            return (
              <div key={dbKey} className={`schema-module schema-module-${status.toLowerCase()}`}>
                <div className="schema-module-header" onClick={() => toggleDb(dbKey)}>
                  <span className="schema-module-icon">{statusIcon(status)}</span>
                  <span className="schema-module-name">{target.target_db.toUpperCase()}</span>
                  <span className="schema-module-files">
                    <span className="schema-file-tag">{target.tables_file}</span>
                    <span className="schema-file-tag">{target.indexes_file}</span>
                  </span>
                  <span className="schema-module-status">{statusLabel(status)}</span>
                  <span className="schema-module-toggle">{expanded ? '▼' : '▶'}</span>
                </div>

                <div className="schema-module-progress">
                  <div className="progress-bar">
                    <div
                      className={`progress-fill ${status === 'Completed' ? 'progress-done' : ''}`}
                      style={{ width: `${progressVal}%` }}
                    />
                  </div>
                  {totalTables > 0 && (
                    <span className="schema-db-tables">
                      {completedTables}/{totalTables} 表 ({Math.round(progressVal)}%)
                    </span>
                  )}
                </div>

                {p?.error_message && (
                  <div className="schema-db-error">{p.error_message}</div>
                )}

                {expanded && (
                  <div className="schema-module-body">
                    <div className="schema-log-header">
                      <h3>SQL 执行日志</h3>
                      {initializing && status === 'Running' && <span className="spinner-sm" />}
                      <button
                        className="btn-copy-log"
                        onClick={(e) => {
                          e.stopPropagation();
                          copyDbLogs(dbKey);
                        }}
                        title="复制日志"
                      >
                        📋 复制
                      </button>
                      <button
                        className="btn-copy-log"
                        onClick={(e) => {
                          e.stopPropagation();
                          exportDbLogs(dbKey);
                        }}
                        title="导出日志"
                      >
                        💾 导出
                      </button>
                    </div>
                    <div
                      className="schema-log-content"
                      ref={(el) => {
                        logContainerRefs.current[dbKey] = el;
                      }}
                      onScroll={() => {
                        const el = logContainerRefs.current[dbKey];
                        if (!el) return;
                        const isNearBottom =
                          el.scrollHeight - el.scrollTop - el.clientHeight < 60;
                        userScrolledUpRefs.current[dbKey] = !isNearBottom;
                      }}
                    >
                      {logs.length === 0 ? (
                        <div className="schema-log-empty">暂无日志</div>
                      ) : (
                        logs.map((line, i) => (
                          <div key={i} className={`schema-log-line ${logLineClass(line)}`}>
                            {line}
                          </div>
                        ))
                      )}
                      <div
                        ref={(el) => {
                          logEndRefs.current[dbKey] = el;
                        }}
                      />
                    </div>
                  </div>
                )}
              </div>
            );
          })}
        </div>
      )}

      <div className="actions">
        <button
          className="btn btn-primary btn-lg"
          onClick={handleInit}
          disabled={
            !selectedConfig ||
            !schemaFolderPath ||
            schemaTargets.length === 0 ||
            loading ||
            scanning ||
            !diffReport ||
            diffReport.has_errors ||
            diffReport.executable_change_count === 0 ||
            initializing
          }
        >
          {initializing ? '⏳ 初始化中...' : '🔧 执行数据库初始化'}
        </button>
        {globalLogs.length > 0 && (
          <button className="btn btn-secondary" onClick={exportAllLogs} disabled={initializing}>
            💾 导出全部日志
          </button>
        )}
      </div>
    </div>
  );
}
