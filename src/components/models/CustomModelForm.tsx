import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { modelsApi, endpointsApi } from '../../lib/api';
import { useState } from 'react';

const CAPABILITIES = [
  'chat',
  'responses',
  'embeddings',
  'images',
  'audio',
  'streaming',
  'tool_calling',
  'vision_input',
];

export function CustomModelForm({ onSaved }: { onSaved: () => void }) {
  const queryClient = useQueryClient();
  const { data: endpoints = [] } = useQuery({
    queryKey: ['endpoints'],
    queryFn: endpointsApi.list,
  });

  const [endpointId, setEndpointId] = useState('');
  const [modelName, setModelName] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [contextWindow, setContextWindow] = useState<number | ''>('');
  const [caps, setCaps] = useState<string[]>([]);

  const create = useMutation({
    mutationFn: () =>
      modelsApi.createCustom({
        endpoint_id: endpointId,
        model_name: modelName,
        display_name: displayName || undefined,
        capabilities: caps.length ? caps : undefined,
        context_window: contextWindow === '' ? undefined : contextWindow,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['models'] });
      setModelName('');
      setDisplayName('');
      setCaps([]);
      setContextWindow('');
      onSaved();
    },
    onError: (e: Error) => alert(`创建失败: ${e.message}`),
  });

  const toggleCap = (c: string) =>
    setCaps((prev) => (prev.includes(c) ? prev.filter((x) => x !== c) : [...prev, c]));

  return (
    <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 space-y-3">
      <h2 className="font-semibold">添加自定义模型</h2>
      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="block text-xs text-gray-500 mb-1">绑定端点</label>
          <select
            value={endpointId}
            onChange={(e) => setEndpointId(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          >
            <option value="">请选择端点</option>
            {endpoints.map((ep) => (
              <option key={ep.id} value={ep.id}>
                {ep.name}
              </option>
            ))}
          </select>
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">模型名</label>
          <input
            value={modelName}
            onChange={(e) => setModelName(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
            placeholder="如 gpt-4o-mini"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">显示名（可选）</label>
          <input
            value={displayName}
            onChange={(e) => setDisplayName(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">上下文窗口（可选）</label>
          <input
            type="number"
            value={contextWindow}
            onChange={(e) => setContextWindow(e.target.value === '' ? '' : Number(e.target.value))}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          />
        </div>
      </div>
      <div>
        <label className="block text-xs text-gray-500 mb-1">能力类型</label>
        <div className="flex flex-wrap gap-2">
          {CAPABILITIES.map((c) => (
            <button
              key={c}
              onClick={() => toggleCap(c)}
              className={`px-2 py-1 rounded text-xs border ${
                caps.includes(c)
                  ? 'bg-blue-600 text-white border-blue-600'
                  : 'bg-transparent text-gray-600 dark:text-gray-400 border-gray-300 dark:border-gray-700'
              }`}
            >
              {c}
            </button>
          ))}
        </div>
      </div>
      <div className="flex gap-2">
        <button
          onClick={() => create.mutate()}
          disabled={!endpointId || !modelName || create.isPending}
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
