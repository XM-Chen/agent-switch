import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { accountsApi, authApi, type Account } from '../lib/api';
import { useState } from 'react';

export function AccountsPage() {
  const queryClient = useQueryClient();
  const { data: accounts = [], isLoading, error } = useQuery({
    queryKey: ['accounts'],
    queryFn: accountsApi.list,
  });

  const [showForm, setShowForm] = useState(false);

  const codexLogin = useMutation({
    mutationFn: authApi.startCodexLogin,
    onSuccess: (data) => {
      window.open(data.auth_url, '_blank');
    },
    onError: (e: Error) => alert(`启动登录失败: ${e.message}`),
  });

  const removeAccount = useMutation({
    mutationFn: (id: string) => accountsApi.delete(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['accounts'] }),
  });

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">账号</h1>
          <p className="text-sm text-gray-500 mt-1">管理上游供应商账号与认证</p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => codexLogin.mutate()}
            disabled={codexLogin.isPending}
            className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
          >
            {codexLogin.isPending ? '启动中...' : 'Codex OAuth 登录'}
          </button>
          <button
            onClick={() => setShowForm((v) => !v)}
            className="px-4 py-2 bg-gray-100 dark:bg-gray-800 rounded-md text-sm hover:bg-gray-200 dark:hover:bg-gray-700"
          >
            {showForm ? '取消' : '添加 API Key 账号'}
          </button>
        </div>
      </div>

      {showForm && <ApiKeyAccountForm onSaved={() => setShowForm(false)} />}

      {isLoading && <p className="text-gray-500">加载中...</p>}
      {error && <p className="text-red-500">加载失败: {error.message}</p>}

      <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 overflow-hidden">
        <table className="w-full text-sm">
          <thead className="bg-gray-50 dark:bg-gray-800/50 text-gray-600 dark:text-gray-400">
            <tr>
              <th className="text-left px-4 py-3">名称</th>
              <th className="text-left px-4 py-3">平台</th>
              <th className="text-left px-4 py-3">授权方式</th>
              <th className="text-left px-4 py-3">优先级</th>
              <th className="text-left px-4 py-3">状态</th>
              <th className="text-left px-4 py-3">凭据</th>
              <th className="text-left px-4 py-3">操作</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-100 dark:divide-gray-800">
            {accounts.length === 0 && (
              <tr>
                <td colSpan={7} className="px-4 py-8 text-center text-gray-400">
                  暂无账号，请通过 Codex OAuth 登录或添加 API Key 账号。
                </td>
              </tr>
            )}
            {accounts.map((a: Account) => (
              <tr key={a.id}>
                <td className="px-4 py-3 font-medium">{a.name}</td>
                <td className="px-4 py-3">{a.platform}</td>
                <td className="px-4 py-3">
                  {a.account_type === 'oauth_codex' ? 'OAuth Codex' : 'API Key'}
                </td>
                <td className="px-4 py-3">{a.priority}</td>
                <td className="px-4 py-3">
                  <StatusBadge status={a.status} />
                </td>
                <td className="px-4 py-3">
                  {a.has_credentials ? '已配置' : '未配置'}
                </td>
                <td className="px-4 py-3">
                  <button
                    onClick={() => {
                      if (confirm(`确定删除账号 "${a.name}"？`)) removeAccount.mutate(a.id);
                    }}
                    className="text-red-500 hover:text-red-600 text-xs"
                  >
                    删除
                  </button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
    </div>
  );
}

function StatusBadge({ status }: { status: string }) {
  const color =
    status === 'active'
      ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
      : status === 'error' || status === 'expired'
      ? 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400'
      : 'bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400';
  return (
    <span className={`px-2 py-0.5 rounded text-xs ${color}`}>{status}</span>
  );
}

function ApiKeyAccountForm({ onSaved }: { onSaved: () => void }) {
  const queryClient = useQueryClient();
  const [name, setName] = useState('');
  const [platform, setPlatform] = useState('custom');
  const [apiKey, setApiKey] = useState('');

  const create = useMutation({
    mutationFn: () =>
      accountsApi.create({
        name,
        account_type: 'apikey',
        platform,
        api_key: apiKey || undefined,
        priority: 0,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['accounts'] });
      setName('');
      setPlatform('custom');
      setApiKey('');
      onSaved();
    },
    onError: (e: Error) => alert(`创建失败: ${e.message}`),
  });

  return (
    <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 space-y-3">
      <h2 className="font-semibold">添加 API Key 账号</h2>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="block text-xs text-gray-500 mb-1">名称</label>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
            placeholder="例如：备用 OpenAI 端点"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">平台</label>
          <input
            value={platform}
            onChange={(e) => setPlatform(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          />
        </div>
      </div>
      <div>
        <label className="block text-xs text-gray-500 mb-1">API Key</label>
        <input
          type="password"
          value={apiKey}
          onChange={(e) => setApiKey(e.target.value)}
          className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
          placeholder="sk-..."
        />
      </div>
      <div className="flex gap-2">
        <button
          onClick={() => create.mutate()}
          disabled={!name || create.isPending}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
        >
          {create.isPending ? '创建中...' : '创建'}
        </button>
        <button
          onClick={onSaved}
          className="px-4 py-2 bg-gray-100 dark:bg-gray-800 rounded-md text-sm"
        >
          取消
        </button>
      </div>
    </div>
  );
}
