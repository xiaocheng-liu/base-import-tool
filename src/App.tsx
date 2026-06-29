import { useState } from 'react';
import './App.css';
import DbConfigManager from './components/DbConfigManager';
import ImportManager from './components/ImportManager';
import SchemaInit from './components/SchemaInit';
import type { CsvFileInfo, DbConfig } from './types';

type Tab = 'config' | 'schema' | 'import';

function App() {
  const [activeTab, setActiveTab] = useState<Tab>('config');
  const [csvFiles, setCsvFiles] = useState<CsvFileInfo[]>([]);
  const [dbConfigs, setDbConfigs] = useState<DbConfig[]>([]);
  const [selectedDbConfigId, setSelectedDbConfigId] = useState<string>('');
  const [selectedFiles, setSelectedFiles] = useState<Set<string>>(new Set());

  const tabs: { key: Tab; label: string; icon: string }[] = [
    { key: 'config', label: '1 数据库连接配置', icon: '⚙️' },
    { key: 'schema', label: '2 数据库初始化', icon: '🏗️' },
    { key: 'import', label: '3 数据导入', icon: '🚀' },
  ];

  return (
    <div className="app">
      <header className="app-header">
        <h1>数据库导入工具</h1>
      </header>

      <nav className="tab-nav">
        {tabs.map((tab) => (
          <button
            key={tab.key}
            className={`tab-btn ${activeTab === tab.key ? 'active' : ''}`}
            onClick={() => setActiveTab(tab.key)}
          >
            <span className="tab-icon">{tab.icon}</span>
            {tab.label}
          </button>
        ))}
      </nav>

      <main className="app-main">
        <div style={{ display: activeTab === 'config' ? 'block' : 'none' }}>
          <DbConfigManager
            dbConfigs={dbConfigs}
            setDbConfigs={setDbConfigs}
            selectedDbConfigId={selectedDbConfigId}
            setSelectedDbConfigId={setSelectedDbConfigId}
          />
        </div>
        <div style={{ display: activeTab === 'schema' ? 'block' : 'none' }}>
          <SchemaInit
            dbConfigs={dbConfigs}
            selectedDbConfigId={selectedDbConfigId}
            setSelectedDbConfigId={setSelectedDbConfigId}
          />
        </div>
        <div style={{ display: activeTab === 'import' ? 'block' : 'none' }}>
          <ImportManager
            csvFiles={csvFiles}
            setCsvFiles={setCsvFiles}
            selectedFiles={selectedFiles}
            setSelectedFiles={setSelectedFiles}
            dbConfigs={dbConfigs}
            selectedDbConfigId={selectedDbConfigId}
          />
        </div>
      </main>
    </div>
  );
}

export default App;
