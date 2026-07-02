import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { aliasesApi, endpointsApi, type AliasItem } from '../../lib/api';
import { useState } from 'react';

const SCOPE_TYPES = ['global', 'tool', 'route', 'endpoint'];

export function AliasPanel() {
  const queryClient = useQueryClient();
  const { data: aliases = [], isLoading, error } = useQuery({
    queryKey: ['aliases'],
    queryFn: () => aliasesApi.list(),
  });

  const [showForm, setShowForm] = useState(false);
  const [resolveInput, setResolveInput] = useState('');
  const [resolveResult, setResolveResult] = useState<string | null>(null);

  const remove = useMutation({
    mutationFn: (id: string) => aliasesApi.delete(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['aliases'] }),
  });

  const resolve = useMutation({
    mutationFn: () => aliasesApi.resolve(resolveInput),
    onSuccess: (r) => {
      if (r.candidates.length === 0) {
        setResolveResult(`未匹配：matched_scope=${r.matched_scope}`);
      } else {
        const lines = r.candidates.map(
          (c) =>
            `  → ${c.model_name}${c.endpoint_id ? ` @ ${c.endpoint_id.slice(0, 8)}` : ''} (priority=${c.priority}, ${c.is_valid ? '有效' : `失效:${c.invalid_reason}`})`,
        );
        setResolveResult(`匹配范围：${r.matched_scope}\n${lines.join('\n')}`);
      }
    },
    onError: (e: Error) => setResolveResult(`解析失败：${e.message}`),
  });

  return (
    <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 space-y-4">
      <div className="flex items-center justify-between">
        <div>
          <h2 className="font-semibold">别名映射</h2>
          <p className="text-xs text-gray-500 mt-0.5">将本地别名映射到具体端点模型，支持分作用域优先级</p>
        </div>
        <button
          onClick={() => setShowForm((v) => !v)}
          className="px-3 py-1.5 bg-blue-600 text-white rounded-md text-xs hover:bg-blue-700"
        >
          {showForm ? '取消' : '添加别名'}
        </button>
      </div>

      {showForm && <AliasForm onSaved={() => setShowForm(false)} />}

      {isLoading && <p className="text-gray-500 text-sm">加载中...</p>}
      {error && <p className="text-red-500 text-sm">加载失败: {error.message}</p>}

      <div className="space-y-1">
        {aliases.length === 0 && <p className="text-gray-400 text-sm text-center py-4">暂无别名</p>}
        {aliases.map((a: AliasItem) => (
          <div
            key={a.id}
            className="flex items-center justify-between px-3 py-2 bg-gray-50 dark:bg-gray-800/50 rounded text-sm"
          >
            <div className="flex items-center gap-2 flex-wrap">
              <span className="font-mono font-medium">{a.alias_name}</span>
              <span className="px-1.5 py-0.5 bg-indigo-100 dark:bg-indigo-900/30 text-indigo-600 dark:text-indigo-400 rounded text-xs">
                {a.scope_type}
                {a.scope_id ? `:${a.scope_id.slice(0, 8)}` : ''}
              </span>
              <span className="text-gray-500 text-xs">→ {a.target_model_name}</span>
              {a.target_endpoint_id && (
                <span className="text-gray-400 text-xs font-mono">@ {a.target_endpoint_id.slice(0, 8)}</span>
              )}
              <span className="text-gray-400 text-xs">p={a.priority}</span>
              {a.invalid_reason && (
                <span className="text-red-500 text-xs">失效：{a.invalid_reason}</span>
              )}
            </div>
            <button
              onClick={() => remove.mutate(a.id)}
              className="text-red-500 hover:text-red-600 text-xs"
            >
              删除
            </button>
          </div>
        ))}
      </div>

      <div className="border-t border-gray-200 dark:border-gray-800 pt-4 space-y-2">
        <h3 className="text-sm font-medium">解析测试</h3>
        <div className="flex gap-2">
          <input
            value={resolveInput}
            onChange={(e) => setResolveInput(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && resolveInput && !resolve.isPending) resolve.mutate();
            }}
            className="flex-1 px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
            placeholder="输入别名，如 sonnet"
          />
          <button
            onClick={() => resolve.mutate()}
            disabled={!resolveInput || resolve.isPending}
            className="px-4 py-2 bg-gray-700 text-white rounded-md text-sm hover:bg-gray-800 disabled:opacity-50"
          >
            {resolve.isPending ? '解析中...' : '解析'}
          </button>
        </div>
        {resolveResult && (
          <pre className="bg-gray-50 dark:bg-gray-800/50 rounded p-3 text-xs whitespace-pre-wrap font-mono">
            {resolveResult}
          </pre>
        )}
      </div>
    </div>
  );
}

function AliasForm({ onSaved }: { onSaved: () => void }) {
  const queryClient = useQueryClient();
  const { data: endpoints = [] } = useQuery({
    queryKey: ['endpoints'],
    queryFn: endpointsApi.list,
  });

  const [scopeType, setScopeType] = useState('global');
  const [scopeId, setScopeId] = useState('');
  const [aliasName, setAliasName] = useState('');
  const [targetEndpointId, setTargetEndpointId] = useState('');
  const [targetModelName, setTargetModelName] = useState('');
  const [priority, setPriority] = useState(0);

  const create = useMutation({
    mutationFn: () =>
      aliasesApi.create({
        scope_type: scopeType,
        scope_id: scopeId || (scopeType === 'global' ? null : undefined),
        alias_name: aliasName,
        target_endpoint_id: targetEndpointId || null,
        target_model_name: targetModelName,
        priority,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['aliases'] });
      setAliasName('');
      setTargetModelName('');
      setScopeId('');
      setTargetEndpointId('');
      onSaved();
    },
    onError: (e: Error) => alert(`创建失败: ${e.message}`),
  });

  return (
    <div className="border border-gray-200 dark:border-gray-800 rounded-md p-3 space-y-3">
      <div className="grid grid-cols-3 gap-3">
        <div>
          <label className="block text-xs text-gray-500 mb-1">作用域</label>
          <select
            value={scopeType}
            onChange={(e) => setScopeType(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          >
            {SCOPE_TYPES.map((s) => (
              <option key={s} value={s}>
                {s}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">作用域 ID（global 留空）</label>
          <input
            value={scopeId}
            onChange={(e) => setScopeId(e.target.value)}
            disabled={scopeType === 'global'}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent disabled:opacity-50"
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
        <div>
          <label className="block text-xs text-gray-500 mb-1">别名名</label>
          <input
            value={aliasName}
            onChange={(e) => setAliasName(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
            placeholder="如 sonnet"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">目标端点（可选）</label>
          <select
            value={targetEndpointId}
            onChange={(e) => setTargetEndpointId(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          >
            <option value="">不绑定端点</option>
            {endpoints.map((ep) => (
              <option key={ep.id} value={ep.id}>
                {ep.name}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">目标模型名</label>
          <input
            value={targetModelName}
            onChange={(e) => setTargetModelName(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
            placeholder="如 claude-sonnet-4-6"
          />
        </div>
      </div>
      <div className="flex gap-2">
        <button
          onClick={() => create.mutate()}
          disabled={!aliasName || !targetModelName || create.isPending}
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
