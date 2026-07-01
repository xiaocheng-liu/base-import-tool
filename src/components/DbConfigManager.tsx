import { useEffect, useState } from 'react';
import type { DbConfig, DbType, OracleConnectionMode, ConnectionTestResult } from '../types';

interface Props {
  dbConfigs: DbConfig[];
  setDbConfigs: (configs: DbConfig[]) => void;
  selectedDbConfigId: string;
  setSelectedDbConfigId: (id: string) => void;
}

const DB_TYPES: DbType[] = ['Oracle', 'DM', 'PostgreSQL', 'MySQL'];

const DEFAULT_PORTS: Record<DbType, number> = {
  Oracle: 1521,
  DM: 5236,
  PostgreSQL: 5432,
  MySQL: 3306,
};

const emptyConfig = (): DbConfig => ({
  id: '',
  db_type: 'Oracle',
  host: 'localhost',
  port: 1521,
  username: 'cbs',
  password: 'pdms',
  database: 'orcl',
  oracle_connection_mode: 'ServiceName',
  extra_params: '',
});

export default function DbConfigManager({
  dbConfigs,
  setDbConfigs,
  selectedDbConfigId,
  setSelectedDbConfigId,
}: Props) {
  const existingConfig = dbConfigs.length > 0 ? dbConfigs[0] : null;
  const [editing, setEditing] = useState<DbConfig>(existingConfig || emptyConfig());
  const [testResult, setTestResult] = useState<ConnectionTestResult | null>(null);
  const [testing, setTesting] = useState(false);
  const [saving, setSaving] = useState(false);

  // 当 dbConfigs 变化时同步 editing
  useEffect(() => {
    if (dbConfigs.length > 0) {
      setEditing({ ...dbConfigs[0] });
      if (!selectedDbConfigId) {
        setSelectedDbConfigId(dbConfigs[0].id);
      }
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [dbConfigs.length]);

  const handleSave = async () => {
    setSaving(true);
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const saved: DbConfig = await invoke('save_db_config', {
        config: editing,
      });
      setDbConfigs([saved]);
      setSelectedDbConfigId(saved.id);
      setTestResult(null);
    } catch (e: any) {
      setTestResult({ success: false, message: `保存失败: ${e}`, db_version: null });
    } finally {
      setSaving(false);
    }
  };

  const handleDelete = async () => {
    if (!existingConfig) return;
    try {
      const { invoke } = await import('@tauri-apps/api/core');
      await invoke('delete_db_config', { id: existingConfig.id });
      setDbConfigs([]);
      setSelectedDbConfigId('');
      setEditing(emptyConfig());
      setTestResult(null);
    } catch (e: any) {
      setTestResult({ success: false, message: `删除失败: ${e}`, db_version: null });
    }
  };

  const handleTestConnection = async () => {
    setTesting(true);
    setTestResult(null);
    let hasCompleted = false;
    const slowTimer = window.setTimeout(() => {
      if (!hasCompleted) {
        setTestResult({
          success: false,
          message: '连接测试超过 5 秒仍未返回，请检查网络、地址、端口或数据库服务状态',
          db_version: null,
        });
      }
    }, 5000);

    try {
      const { invoke } = await import('@tauri-apps/api/core');
      const result: ConnectionTestResult = await invoke('test_connection', {
        config: editing,
      });
      hasCompleted = true;
      setTestResult(result);
    } catch (e: any) {
      hasCompleted = true;
      setTestResult({ success: false, message: `测试失败: ${e}`, db_version: null });
    } finally {
      window.clearTimeout(slowTimer);
      setTesting(false);
    }
  };

  return (
    <div className="db-config-manager">
      <div className="section-header">
        <h2>数据库连接配置</h2>
        <p>配置数据库连接信息，保存后可在初始化和导入中使用</p>
      </div>

      <div className="form-row">
        <div className="form-group">
          <label>数据库类型</label>
          <select
            value={editing.db_type}
            onChange={(e) =>
              setEditing({
                ...editing,
                db_type: e.target.value as DbType,
                port: DEFAULT_PORTS[e.target.value as DbType],
                database: e.target.value === 'Oracle' ? 'orcl' : '',
              })
            }
          >
            {DB_TYPES.map((t) => (
              <option key={t} value={t}>
                {t}
              </option>
            ))}
          </select>
        </div>
      </div>

      <div className="form-row">
        <div className="form-group">
          <label>主机地址</label>
          <input
            type="text"
            value={editing.host}
            onChange={(e) => setEditing({ ...editing, host: e.target.value })}
            placeholder="localhost"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
          />
        </div>
        <div className="form-group">
          <label>端口</label>
          <input
            type="number"
            value={editing.port}
            onChange={(e) => setEditing({ ...editing, port: Number(e.target.value) })}
          />
        </div>
      </div>

      <div className="form-row">
        <div className="form-group">
          <label>用户名</label>
          <input
            type="text"
            value={editing.username}
            onChange={(e) => setEditing({ ...editing, username: e.target.value })}
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
          />
        </div>
        <div className="form-group">
          <label>密码</label>
          <input
            type="password"
            value={editing.password}
            onChange={(e) => setEditing({ ...editing, password: e.target.value })}
          />
        </div>
      </div>

      {editing.db_type === 'Oracle' && (
        <div className="form-row">
          <div className="form-group">
            <label>连接模式</label>
            <select
              value={editing.oracle_connection_mode || 'ServiceName'}
              onChange={(e) =>
                setEditing({
                  ...editing,
                  oracle_connection_mode: e.target.value as OracleConnectionMode,
                })
              }
            >
              <option value="ServiceName">Service Name</option>
              <option value="SID">SID</option>
            </select>
          </div>
          <div className="form-group">
            <label>{editing.oracle_connection_mode === 'SID' ? 'SID' : 'Service Name'}</label>
            <input
              type="text"
              value={editing.database}
              onChange={(e) => setEditing({ ...editing, database: e.target.value })}
              placeholder={editing.oracle_connection_mode === 'SID' ? '例如: ORCL' : '例如: orcl.example.com'}
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
            />
          </div>
        </div>
      )}

      {editing.db_type !== 'Oracle' && editing.db_type !== 'DM' && (
        <div className="form-row">
          <div className="form-group">
            <label>数据库名</label>
            <input
              type="text"
              value={editing.database}
              onChange={(e) => setEditing({ ...editing, database: e.target.value })}
              placeholder="例如: mydb"
              autoCapitalize="none"
              autoCorrect="off"
              spellCheck={false}
            />
          </div>
        </div>
      )}

      <div className="form-row">
        <div className="form-group">
          <label>额外参数</label>
          <input
            type="text"
            value={editing.extra_params}
            onChange={(e) => setEditing({ ...editing, extra_params: e.target.value })}
            placeholder="可选"
            autoCapitalize="none"
            autoCorrect="off"
            spellCheck={false}
          />
        </div>
      </div>

      {testResult && (
        <div className={`test-result ${testResult.success ? 'success' : 'fail'}`}>
          {testResult.message}
          {testResult.db_version && (
            <span className="db-version"> | {testResult.db_version}</span>
          )}
        </div>
      )}

      <div className="form-actions">
        <button
          className="btn btn-secondary"
          onClick={handleTestConnection}
          disabled={testing}
        >
          {testing ? '测试中...' : '测试连接'}
        </button>
        <button className="btn btn-primary" onClick={handleSave} disabled={saving}>
          {saving ? '保存中...' : '保存配置'}
        </button>
        {existingConfig && (
          <button className="btn btn-danger" onClick={handleDelete}>
            删除配置
          </button>
        )}
      </div>


    </div>
  );
}
