import { useState, useEffect, useCallback, useRef } from 'react';
import FileScanner from './FileScanner';
import type { CsvFileInfo, DbConfig, ImportTask, ImportProgress } from '../types';

interface Props {
  csvFiles: CsvFileInfo[];
  setCsvFiles: (files: CsvFileInfo[]) => void;
  selectedFiles: Set<string>;
  setSelectedFiles: (files: Set<string>) => void;
  dbConfigs: DbConfig[];
  selectedDbConfigId: string;
}

export default function ImportManager({
  csvFiles,
  setCsvFiles,
  selectedFiles,
  setSelectedFiles,
  dbConfigs,
  selectedDbConfigId,
}: Props) {
  const [tasks, setTasks] = useState<ImportTask[]>([]);
  const [importing, setImporting] = useState(false);
  const [truncateFirst, setTruncateFirst] = useState(false);
  const [logs, setLogs] = useState<string[]>([]);
  const [schemaDir, setSchemaDir] = useState('');
  // 失败项展开状态：key=taskId-errorIndex，true=展开
  const [expandedErrors, setExpandedErrors] = useState<Record<string, boolean>>({});
  const logEndRef = useRef<HTMLDivElement>(null);
  const logContainerRef = useRef<HTMLDivElement>(null);
  const userScrolledUpRef = useRef(false);
  const pollingRef = useRef<number | null>(null);

  const handleCopyLogs = () => {
    const text = logs.join('\n');
    navigator.clipboard.writeText(text).then(() => {
    }).catch(() => {});
  };

  const handleExportLogs = () => {
    const text = logs.join('\n');
    const blob = new Blob([text], { type: 'text/plain;charset=utf-8' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    const now = new Date();
    const ts = `${now.getFullYear()}${String(now.getMonth()+1).padStart(2,'0')}${String(now.getDate()).padStart(2,'0')}_${String(now.getHours()).padStart(2,'0')}${String(now.getMinutes()).padStart(2,'0')}${String(now.getSeconds()).padStart(2,'0')}`;
    a.href = url;
    a.download = `import_log_${ts}.txt`;
    a.click();
    URL.revokeObjectURL(url);
  };
  const tasksRef = useRef<ImportTask[]>([]);

  // 只有用户在底部附近时才自动滚动到底部，否则尊重用户的手动滚动位置
  useEffect(() => {
    const container = logContainerRef.current;
    if (!container) return;
    // 判断用户是否在底部附近（阈值 60px），如果在底部则自动滚到底
    const isNearBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 60;
    if (isNearBottom || !userScrolledUpRef.current) {
      logEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [logs]);

  // 同步 tasks 到 ref，避免 pollProgress 的陈旧闭包问题
  tasksRef.current = tasks;

  const selectedCsvFiles = csvFiles.filter((f) => selectedFiles.has(f.file_path));
  const selectedConfig = dbConfigs.find((c) => c.id === selectedDbConfigId);

  const resetImportState = () => {
    if (pollingRef.current) {
      clearInterval(pollingRef.current);
      pollingRef.current = null;
    }
    tasksRef.current = [];
    setTasks([]);
    setImporting(false);
  };

  // 轮询导入进度 — 不依赖 tasks，使用 ref 获取最新值
  const pollProgress = useCallback(async () => {
    const currentTasks = tasksRef.current;
    if (currentTasks.length === 0) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const taskIds = currentTasks.map((t) => t.id);
      const progress: Record<string, ImportProgress> = await invoke('get_import_progress', {
        taskIds,
      });

      setTasks((prev) =>
        prev.map((task) => {
          const p = progress[task.id];
          if (p) {
            return {
              ...task,
              status: p.status,
              progress: p.progress,
              total_rows: p.total_rows,
              imported_rows: p.imported_rows,
              error_message: p.error_message,
              errors: p.errors ?? task.errors,
            };
          }
          return task;
        })
      );

      // 检查是否全部完成
      const allDone = Object.values(progress).every(
        (p) => p.status === 'Completed' || p.status === 'Failed'
      );
      if (allDone && pollingRef.current) {
        clearInterval(pollingRef.current);
        pollingRef.current = null;
        setImporting(false);
      }
    } catch (e) {
      console.error('获取进度失败:', e);
    }
  }, []);

  useEffect(() => {
    if (importing) {
      // 先清除旧的 interval，避免重复
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
      }
      pollingRef.current = window.setInterval(pollProgress, 500);
    }
    return () => {
      if (pollingRef.current) {
        clearInterval(pollingRef.current);
        pollingRef.current = null;
      }
    };
  }, [importing, pollProgress]);

  const handleStartImport = async () => {
    if (!selectedConfig) return;

    setImporting(true);
    setLogs([]);
    userScrolledUpRef.current = false;

    // 使用 ref 缓存日志，批量更新减少渲染次数
    const logBuffer: string[] = [];
    let flushTimer: ReturnType<typeof setInterval> | null = null;

    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<string>('import-log', (event) => {
      logBuffer.push(event.payload);
      if (!flushTimer) {
        flushTimer = setInterval(() => {
          if (logBuffer.length > 0) {
            const batch = logBuffer.splice(0);
            setLogs((prev) => [...prev, ...batch]);
          }
        }, 100);
      }
    });

    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const result: ImportTask[] = await invoke('start_import', {
        csvFiles: selectedCsvFiles,
        dbConfigId: selectedConfig.id,
      });
      setTasks(result);
    } catch (e: any) {
      setLogs((prev) => [...prev, `✗ 导入失败: ${e}`]);
      setImporting(false);
    }

    // 轮询进度并检测完成，完成后清理
    const checkDone = setInterval(async () => {
      const currentTasks = tasksRef.current;
      if (currentTasks.length === 0) return;
      try {
        const { invoke } = await import('@tauri-apps/api/core');
        const taskIds = currentTasks.map((t) => t.id);
        const progress: Record<string, ImportProgress> = await invoke('get_import_progress', {
          taskIds,
        });
        const allDone = Object.values(progress).every(
          (p) => p.status === 'Completed' || p.status === 'Failed'
        );
        if (allDone) {
          // 等一会确保最后的日志都收到了
          setTimeout(() => {
            if (flushTimer) clearInterval(flushTimer);
            if (logBuffer.length > 0) {
              const batch = logBuffer.splice(0);
              setLogs((prev) => [...prev, ...batch]);
            }
            unlisten();
            if (pollingRef.current) {
              clearInterval(pollingRef.current);
              pollingRef.current = null;
            }
            setImporting(false);
          }, 500);
          clearInterval(checkDone);
        }
      } catch (_) {}
    }, 1000);
  };

  const getStatusIcon = (status: string) => {
    switch (status) {
      case 'Pending':
        return '⏳';
      case 'Running':
        return '🔄';
      case 'Completed':
        return '✅';
      case 'Failed':
        return '❌';
      case 'Skipped':
        return '⏭️';
      default:
        return '❓';
    }
  };

  const getStatusLabel = (status: string) => {
    switch (status) {
      case 'Pending':
        return '等待中';
      case 'Running':
        return '导入中';
      case 'Completed':
        return '已完成';
      case 'Failed':
        return '失败';
      case 'Skipped':
        return '已跳过';
      default:
        return status;
    }
  };

  if (!selectedConfig) {
    return (
      <div className="import-manager">
        <div className="section-header">
          <h2>数据导入</h2>
          <p>选择数据目录，自动识别 CSV/SQL 文件并导入</p>
        </div>
        <div className="empty-state">
          <p>请先在「数据库连接配置」中保存连接配置</p>
        </div>
      </div>
    );
  }

  if (selectedCsvFiles.length === 0) {
    return (
      <div className="import-manager">
        <div className="section-header">
          <h2>数据导入</h2>
          <p>选择数据目录，自动识别 CSV/SQL 文件并导入</p>
        </div>
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
        </div>
        <FileScanner
          csvFiles={csvFiles}
          setCsvFiles={setCsvFiles}
          selectedFiles={selectedFiles}
          setSelectedFiles={setSelectedFiles}
          onFilesScanned={resetImportState}
          embedded
          schemaDir={schemaDir || undefined}
        />
      </div>
    );
  }

  const completedCount = tasks.filter((t) => t.status === 'Completed').length;
  const failedCount = tasks.filter((t) => t.status === 'Failed').length;

  return (
    <div className="import-manager">
      <div className="section-header">
        <h2>数据导入</h2>
        <p>选择数据目录，自动识别 CSV/SQL 文件并导入</p>
      </div>

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
        {selectedCsvFiles.length > 0 && (
          <div className="summary-card">
            <span className="summary-label">选中文件</span>
            <span className="summary-value">{selectedCsvFiles.length}</span>
          </div>
        )}
        {tasks.length > 0 && (
          <>
            <div className="summary-card success">
              <span className="summary-label">已完成</span>
              <span className="summary-value">{completedCount}</span>
            </div>
            <div className="summary-card danger">
              <span className="summary-label">失败</span>
              <span className="summary-value">{failedCount}</span>
            </div>
          </>
        )}
      </div>

      <div className="folder-selector">
        <button
          className="btn btn-sm"
          onClick={async () => {
            const { open } = await import('@tauri-apps/plugin-dialog');
            const selected = await open({ directory: true, multiple: false, title: '选择 02_schema 文件夹（用于查看表注释）' });
            if (selected && typeof selected === 'string') setSchemaDir(selected);
          }}
          title="用于查看 SQL 文件的字段注释"
        >
          📂 选择 Schema 目录（可选）
        </button>
        {schemaDir && <span className="folder-path" title={schemaDir}>{schemaDir}</span>}
      </div>

      <FileScanner
        csvFiles={csvFiles}
        setCsvFiles={setCsvFiles}
        selectedFiles={selectedFiles}
        setSelectedFiles={setSelectedFiles}
        onFilesScanned={resetImportState}
        embedded
        schemaDir={schemaDir || undefined}
      />

      {!importing && selectedCsvFiles.length > 0 && (
        <div className="actions import-actions-row">
          <label className="truncate-option">
            <input
              type="checkbox"
              checked={truncateFirst}
              onChange={(e) => setTruncateFirst(e.target.checked)}
            />
            <span>导入前先清空目标表数据</span>
          </label>
          <button className="btn btn-primary btn-lg" onClick={handleStartImport}>
            🚀 开始导入
          </button>
        </div>
      )}

      {logs.length > 0 && (
        <div className="schema-log-viewer">
          <div className="schema-log-header">
            <h3>SQL 执行日志</h3>
            {importing && <span className="spinner-sm" />}
            <button className="btn-copy-log" onClick={handleCopyLogs} title="复制日志">
              📋 复制
            </button>
            <button className="btn-copy-log" onClick={handleExportLogs} title="导出日志">
              💾 导出
            </button>
          </div>
          <div
            className="schema-log-content"
            ref={logContainerRef}
            onScroll={() => {
              const el = logContainerRef.current;
              if (!el) return;
              const isNearBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 60;
              userScrolledUpRef.current = !isNearBottom;
            }}
          >
            {logs.map((line, i) => (
              <div
                key={i}
                className={`schema-log-line ${line.startsWith('✗') ? 'log-error' : line.startsWith('✓') ? 'log-success' : line.startsWith('▶') ? 'log-header' : line.startsWith('═') ? 'log-footer' : line.trimStart().startsWith('SQL:') ? 'log-sql' : ''}`}
              >
                {line}
              </div>
            ))}
            <div ref={logEndRef} />
          </div>
        </div>
      )}

      {tasks.length > 0 && (
        <div className="task-list">
          <h3>导入任务</h3>
          {tasks.map((task) => (
            <div key={task.id} className={`task-item task-${task.status.toLowerCase()}`}>
              <div className="task-header">
                <span className="task-status-icon">{getStatusIcon(task.status)}</span>
                <span className="task-name">
                  [{task.csv_file.target_db}] {task.csv_file.table_name}
                  <span className="task-file-type">
                    {task.csv_file.file_type === 'Sql' ? 'SQL' : 'CSV'}
                  </span>
                </span>
                <span className={`task-status task-status-${task.status.toLowerCase()}`}>
                  {getStatusLabel(task.status)}
                </span>
              </div>

              {task.csv_file.file_type === 'Csv' &&
                (task.status === 'Running' || task.status === 'Completed') && (
                <div className="task-progress">
                  <div className="progress-bar">
                    <div
                      className="progress-fill"
                      style={{ width: `${task.progress}%` }}
                    />
                  </div>
                  <span className="progress-text">
                    {task.imported_rows} / {task.total_rows} 行 ({task.progress.toFixed(1)}%)
                  </span>
                </div>
              )}

              {task.csv_file.file_type === 'Sql' &&
                (task.status === 'Running' || task.status === 'Completed') && (
                <div className="task-sql-status">
                  {task.status === 'Running' ? '正在执行 SQL 文件...' : 'SQL 文件执行完成'}
                </div>
              )}

              {task.status === 'Failed' && (task.errors?.length || task.error_message) && (
                <div className="task-errors">
                  {task.errors && task.errors.length > 0 && (
                    <>
                      <div className="task-errors-header">
                        共 {task.errors.length} 条 SQL 执行失败：
                      </div>
                      <div className="task-errors-list">
                        {task.errors.map((err, _i) => {
                          const key = `${task.id}-${err.index}`;
                          // 默认展开（首次出现时默认 true）
                          const isOpen = expandedErrors[key] ?? true;
                          return (
                            <div
                              key={key}
                              className={`task-error-item ${isOpen ? 'is-open' : 'is-collapsed'}`}
                            >
                              <div
                                className="task-error-item-header"
                                onClick={() =>
                                  setExpandedErrors((prev) => ({ ...prev, [key]: !isOpen }))
                                }
                                role="button"
                                aria-expanded={isOpen}
                                tabIndex={0}
                                onKeyDown={(e) => {
                                  if (e.key === 'Enter' || e.key === ' ') {
                                    e.preventDefault();
                                    setExpandedErrors((prev) => ({ ...prev, [key]: !isOpen }));
                                  }
                                }}
                              >
                                <span className="task-error-index">第 {err.index} 条</span>
                                <span className="task-error-toggle" aria-hidden>
                                  {isOpen ? '▾' : '▸'}
                                </span>
                              </div>
                              {isOpen && (
                                <div className="task-error-item-body">
                                  <div className="task-error-msg">{err.error}</div>
                                  <div className="task-error-sql-label">出错 SQL（完整）：</div>
                                  <pre className="task-error-sql">{err.sql}</pre>
                                  {err.suggestion && (
                                    <div className="task-error-suggestion">
                                      <div className="task-error-suggestion-title">解决建议：</div>
                                      <pre className="task-error-suggestion-body">{err.suggestion}</pre>
                                    </div>
                                  )}
                                </div>
                              )}
                            </div>
                          );
                        })}
                      </div>
                    </>
                  )}
                  {(!task.errors || task.errors.length === 0) && task.error_message && (
                    <div className="task-error">{task.error_message}</div>
                  )}
                </div>
              )}
            </div>
          ))}
        </div>
      )}

      {importing && (
        <div className="import-status-bar">
          <div className="spinner" />
          <span>正在导入数据，请勿关闭窗口...</span>
        </div>
      )}
    </div>
  );
}
