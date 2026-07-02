import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { endpointsApi, type Endpoint } from '../lib/api';
import { useState } from 'react';

export function EndpointsPage() {
  const queryClient = useQueryClient();
  const { data: endpoints = [], isLoading, error } = useQuery({
    queryKey: ['endpoints'],
    queryFn: endpointsApi.list,
  });

  const [showForm, setShowForm] = useState(false);

  const toggle = useMutation({
    mutationFn: ({ id, enabled }: { id: string; enabled: boolean }) =>
      endpointsApi.toggle(id, enabled),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['endpoints'] }),
  });

  const remove = useMutation({
    mutationFn: (id: string) => endpointsApi.delete(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['endpoints'] }),
  });

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">端点</h1>
          <p className="text-sm text-gray-500 mt-1">管理上游端点的 base URL、协议与认证</p>
        </div>
        <button
          onClick={() => setShowForm((v) => !v)}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700"
        >
          {showForm ? '取消' : '添加端点'}
        </button>
      </div>

      {showForm && <EndpointForm onSaved={() => setShowForm(false)} />}

      {isLoading && <p className="text-gray-500">加载中...</p>}
      {error && <p className="text-red-500">加载失败: {error.message}</p>}

      <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 overflow-hidden">
        <table className="w-full text-sm">
          <thead className="bg-gray-50 dark:bg-gray-800/50 text-gray-600 dark:text-gray-400">
            <tr>
              <th className="text-left px-4 py-3">名称</th>
              <th className="text-left px-4 py-3">Base URL</th>
              <th className="text-left px-4 py-3">协议</th>
              <th className="text-left px-4 py-3">认证</th>
              <th className="text-left px-4 py-3">优先级</th>
              <th className="text-left px-4 py-3">启用</th>
              <th className="text-left px-4 py-3">凭据</th>
              <th className="text-left px-4 py-3">操作</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-100 dark:divide-gray-800">
            {endpoints.length === 0 && (
              <tr>
                <td colSpan={8} className="px-4 py-8 text-center text-gray-400">
                  暂无端点，请添加上游端点。
                </td>
              </tr>
            )}
            {endpoints.map((e: Endpoint) => (
              <tr key={e.id}>
                <td className="px-4 py-3 font-medium">{e.name}</td>
                <td className="px-4 py-3 font-mono text-xs text-gray-600 dark:text-gray-400">
                  {e.base_url}
                </td>
                <td className="px-4 py-3">{e.protocol_type}</td>
                <td className="px-4 py-3">{e.auth_mode}</td>
                <td className="px-4 py-3">{e.priority}</td>
                <td className="px-4 py-3">
                  <button
                    onClick={() => toggle.mutate({ id: e.id, enabled: !e.enabled })}
                    disabled={toggle.isPending}
                    className={`px-2 py-0.5 rounded text-xs disabled:opacity-50 disabled:cursor-not-allowed ${
                      e.enabled
                        ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
                        : 'bg-gray-100 text-gray-500 dark:bg-gray-800'
                    }`}
                  >
                    {toggle.isPending ? '切换中...' : e.enabled ? '已启用' : '已禁用'}
                  </button>
                </td>
                <td className="px-4 py-3">
                  {e.has_api_key ? '已配置' : '—'}
                </td>
                <td className="px-4 py-3">
                  <button
                    onClick={() => {
                      if (confirm(`确定删除端点 "${e.name}"？`)) remove.mutate(e.id);
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

const PROTOCOLS = ['anthropic', 'openai_chat', 'openai_responses', 'codex', 'custom'];
const AUTH_MODES = ['apikey', 'oauth_codex', 'none'];

function EndpointForm({ onSaved }: { onSaved: () => void }) {
  const queryClient = useQueryClient();
  const [name, setName] = useState('');
  const [baseUrl, setBaseUrl] = useState('');
  const [protocolType, setProtocolType] = useState('openai_chat');
  const [authMode, setAuthMode] = useState('none');
  const [apiKey, setApiKey] = useState('');
  const [priority, setPriority] = useState(0);

  const create = useMutation({
    mutationFn: () =>
      endpointsApi.create({
        name,
        base_url: baseUrl,
        protocol_type: protocolType,
        auth_mode: authMode,
        api_key: apiKey || undefined,
        priority,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['endpoints'] });
      setName('');
      setBaseUrl('');
      setApiKey('');
      onSaved();
    },
    onError: (e: Error) => alert(`创建失败: ${e.message}`),
  });

  return (
    <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 space-y-3">
      <h2 className="font-semibold">添加端点</h2>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="block text-xs text-gray-500 mb-1">名称</label>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
            placeholder="例如：Anthropic 官方"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">Base URL</label>
          <input
            value={baseUrl}
            onChange={(e) => setBaseUrl(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
            placeholder="https://api.example.com"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">协议</label>
          <select
            value={protocolType}
            onChange={(e) => setProtocolType(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          >
            {PROTOCOLS.map((p) => (
              <option key={p} value={p}>
                {p}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">认证方式</label>
          <select
            value={authMode}
            onChange={(e) => setAuthMode(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          >
            {AUTH_MODES.map((m) => (
              <option key={m} value={m}>
                {m}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">API Key（可选）</label>
          <input
            type="password"
            value={apiKey}
            onChange={(e) => setApiKey(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
            placeholder="sk-..."
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">优先级</label>
          <input
            type="number"
            value={priority}
            onChange={(e) => setPriority(Number(e.target.value))}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          />
        </div>
      </div>
      <div className="flex gap-2">
        <button
          onClick={() => create.mutate()}
          disabled={!name || !baseUrl || create.isPending}
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
