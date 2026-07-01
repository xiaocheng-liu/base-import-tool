import { useEffect, useRef, useState } from 'react';
import type { DbConfig, SchemaTarget } from '../types';

interface Props {
  dbConfigs: DbConfig[];
  selectedDbConfigId: string;
  setSelectedDbConfigId: (id: string) => void;
}

export default function SchemaInit({ dbConfigs, selectedDbConfigId}: Props) {
  const [schemaTargets, setSchemaTargets] = useState<SchemaTarget[]>([]);
  const [schemaFolderPath, setSchemaFolderPath] = useState('');
  const [loading, setLoading] = useState(false);
  const [initializing, setInitializing] = useState(false);
  const [loadError, setLoadError] = useState('');
  const [logs, setLogs] = useState<string[]>([]);
  const logEndRef = useRef<HTMLDivElement>(null);
  const logContainerRef = useRef<HTMLDivElement>(null);
  const userScrolledUpRef = useRef(false);

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
    a.download = `schema_init_log_${ts}.txt`;
    a.click();
    URL.revokeObjectURL(url);
  };

  const selectedConfig = dbConfigs.find((c) => c.id === selectedDbConfigId);

  // 只有用户在底部附近时才自动滚动到底部，否则尊重用户的手动滚动位置
  useEffect(() => {
    const container = logContainerRef.current;
    if (!container) return;
    const isNearBottom = container.scrollHeight - container.scrollTop - container.clientHeight < 60;
    if (isNearBottom || !userScrolledUpRef.current) {
      logEndRef.current?.scrollIntoView({ behavior: 'smooth' });
    }
  }, [logs]);

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
        setLogs([]);
      }
    } catch (e: any) {
      setLoadError(`选择 Schema 文件夹失败: ${e}`);
    }
  };

  const handleClear = () => {
    setSchemaTargets([]);
    setSchemaFolderPath('');
    setLoadError('');
    setLogs([]);
  };

  const handleInit = async () => {
    if (!selectedConfig || schemaTargets.length === 0) return;
    setInitializing(true);
    setLogs([`开始初始化 ${schemaTargets.length} 个库的表结构...`]);
    userScrolledUpRef.current = false;

    // 批量缓存日志，减少 setState 调用
    const logBuffer: string[] = [];
    let flushTimer: ReturnType<typeof setInterval> | null = null;

    const { listen } = await import('@tauri-apps/api/event');
    const unlisten = await listen<string>('schema-log', (event) => {
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
      await invoke('init_all_schemas', {
        dbConfigId: selectedConfig.id,
        schemaDir: schemaFolderPath,
      });
      // 等待 flush 最后一批
      await new Promise((r) => setTimeout(r, 200));
    } catch (e: any) {
      setLogs((prev) => [...prev, `✗ 初始化失败: ${e}`]);
    } finally {
      if (flushTimer) clearInterval(flushTimer);
      if (logBuffer.length > 0) {
        const batch = logBuffer.splice(0);
        setLogs((prev) => [...prev, ...batch]);
      }
      unlisten();
      setInitializing(false);
    }
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
            <span className="folder-path" title={schemaFolderPath}>{schemaFolderPath}</span>
            <button className="btn btn-sm btn-danger" onClick={handleClear}>
              清空
            </button>
          </>
        )}
      </div>

      {schemaTargets.length > 0 && (
        <div className="schema-file-summary">
          {schemaTargets.map((t) => (
            <div key={t.target_db} className="schema-target-info">
              <strong>{t.target_db.toUpperCase()}</strong>
              <span className="schema-file-tag">{t.tables_file}</span>
              <span className="schema-file-tag">{t.indexes_file}</span>
            </div>
          ))}
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
            initializing
          }
        >
          {initializing ? '⏳ 初始化中...' : '🔧 执行数据库初始化'}
        </button>
      </div>

      {logs.length > 0 && (
        <div className="schema-log-viewer">
          <div className="schema-log-header">
            <h3>SQL 执行日志</h3>
            {initializing && <span className="spinner-sm" />}
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
                className={`schema-log-line ${line.startsWith('✗') ? 'log-error' : line.startsWith('✓') ? 'log-success' : line.startsWith('▶') ? 'log-header' : line.startsWith('═') ? 'log-footer' : line.startsWith('初始化完成') ? 'log-summary' : ''}`}
              >
                {line}
              </div>
            ))}
            <div ref={logEndRef} />
          </div>
        </div>
      )}


    </div>
  );
}
