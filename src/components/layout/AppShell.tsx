import { useLocation, useNavigate } from 'react-router-dom';
import { useEffect, useState } from 'react';
import { getVersion } from '@tauri-apps/api/app';
import type { ReactNode } from 'react';

const NAV_ITEMS = [
  { path: '/', label: '总览', icon: '📊' },
  { path: '/accounts', label: '账号', icon: '🔑' },
  { path: '/endpoints', label: '端点', icon: '🔌' },
  { path: '/models', label: '模型', icon: '🧠' },
  { path: '/tools', label: '工具', icon: '🛠️' },
  { path: '/providers', label: '切换器', icon: '🔄' },
  { path: '/mcp', label: 'MCP', icon: '🧩' },
  { path: '/routes', label: '路由', icon: '🔀' },
  { path: '/logs', label: '日志', icon: '📋' },
  { path: '/settings', label: '设置', icon: '⚙️' },
];

interface AppShellProps {
  children: ReactNode;
}

export function AppShell({ children }: AppShellProps) {
  return (
    <div className="flex h-screen">
      <Sidebar />
      <main className="flex-1 overflow-auto bg-gray-50 dark:bg-gray-950 dark:text-gray-100 px-6 py-6">
        {children}
      </main>
    </div>
  );
}

function Sidebar() {
  const location = useLocation();
  const navigate = useNavigate();
  const [version, setVersion] = useState<string>('');

  useEffect(() => {
    getVersion().then(setVersion).catch(() => {});
  }, []);

  return (
    <aside className="w-56 border-r border-gray-200 dark:border-gray-800 bg-white dark:bg-gray-900 flex flex-col">
      <div className="p-4 border-b border-gray-200 dark:border-gray-800">
        <h1 className="text-lg font-bold">Agent-Switch</h1>
        <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">
          管理控制台 {version && `v${version}`}
        </p>
      </div>
      <nav className="flex-1 p-2 space-y-0.5">
        {NAV_ITEMS.map((item) => {
          const isActive = location.pathname === item.path;
          return (
            <button
              key={item.path}
              onClick={() => navigate(item.path)}
              className={`w-full flex items-center gap-3 px-3 py-2 rounded-md text-sm transition-colors ${
                isActive
                  ? 'bg-gray-100 dark:bg-gray-800 font-medium'
                  : 'text-gray-600 dark:text-gray-400 hover:bg-gray-50 dark:hover:bg-gray-800/50'
              }`}
            >
              <span className="text-base">{item.icon}</span>
              <span>{item.label}</span>
            </button>
          );
        })}
      </nav>
    </aside>
  );
}
