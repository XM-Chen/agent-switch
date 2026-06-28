import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { modelsApi, type ModelItem } from '../lib/api';
import { useState } from 'react';
import { AliasPanel } from '../components/models/AliasPanel';
import { CustomModelForm } from '../components/models/CustomModelForm';

export function ModelsPage() {
  const queryClient = useQueryClient();
  const { data: models = [], isLoading, error } = useQuery({
    queryKey: ['models'],
    queryFn: () => modelsApi.list(),
  });

  const [showForm, setShowForm] = useState(false);
  const [lastReport, setLastReport] = useState<string | null>(null);

  const sync = useMutation({
    mutationFn: () => modelsApi.sync(),
    onSuccess: (report) => {
      queryClient.invalidateQueries({ queryKey: ['models'] });
      const ok = report.succeeded.length;
      const fail = report.failed.length;
      const errs = report.errors.length ? `\n错误：${report.errors.join('; ')}` : '';
      setLastReport(`同步完成：成功 ${ok} 个端点，失败 ${fail} 个。${errs}`);
    },
    onError: (e: Error) => setLastReport(`同步失败：${e.message}`),
  });

  const remove = useMutation({
    mutationFn: (id: string) => modelsApi.delete(id),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['models'] }),
  });

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">模型</h1>
          <p className="text-sm text-gray-500 mt-1">端点模型列表、上游刷新与别名映射</p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => sync.mutate()}
            disabled={sync.isPending}
            className="px-4 py-2 bg-green-600 text-white rounded-md text-sm hover:bg-green-700 disabled:opacity-50"
          >
            {sync.isPending ? '同步中...' : '立即刷新全部'}
          </button>
          <button
            onClick={() => setShowForm((v) => !v)}
            className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700"
          >
            {showForm ? '取消' : '添加自定义模型'}
          </button>
        </div>
      </div>

      {lastReport && (
        <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-md p-3 text-sm text-blue-700 dark:text-blue-300 whitespace-pre-wrap">
          {lastReport}
        </div>
      )}

      {showForm && <CustomModelForm onSaved={() => setShowForm(false)} />}

      {isLoading && <p className="text-gray-500">加载中...</p>}
      {error && <p className="text-red-500">加载失败: {error.message}</p>}

      <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 overflow-hidden">
        <table className="w-full text-sm">
          <thead className="bg-gray-50 dark:bg-gray-800/50 text-gray-600 dark:text-gray-400">
            <tr>
              <th className="text-left px-4 py-3">模型名</th>
              <th className="text-left px-4 py-3">显示名</th>
              <th className="text-left px-4 py-3">端点</th>
              <th className="text-left px-4 py-3">来源</th>
              <th className="text-left px-4 py-3">能力</th>
              <th className="text-left px-4 py-3">可用</th>
              <th className="text-left px-4 py-3">操作</th>
            </tr>
          </thead>
          <tbody className="divide-y divide-gray-100 dark:divide-gray-800">
            {models.length === 0 && (
              <tr>
                <td colSpan={7} className="px-4 py-8 text-center text-gray-400">
                  暂无模型，请添加端点后刷新上游。
                </td>
              </tr>
            )}
            {models.map((m: ModelItem) => (
              <tr key={m.id}>
                <td className="px-4 py-3 font-mono text-xs">{m.model_name}</td>
                <td className="px-4 py-3">{m.display_name}</td>
                <td className="px-4 py-3 font-mono text-xs text-gray-600 dark:text-gray-400">
                  {m.endpoint_id.slice(0, 8)}
                </td>
                <td className="px-4 py-3">
                  <span
                    className={`px-2 py-0.5 rounded text-xs ${
                      m.source === 'custom'
                        ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400'
                        : 'bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400'
                    }`}
                  >
                    {m.source}
                  </span>
                </td>
                <td className="px-4 py-3">
                  <div className="flex flex-wrap gap-1">
                    {m.capabilities.map((c) => (
                      <span
                        key={c}
                        className="px-1.5 py-0.5 bg-blue-50 dark:bg-blue-900/20 text-blue-600 dark:text-blue-400 rounded text-xs"
                      >
                        {c}
                      </span>
                    ))}
                  </div>
                </td>
                <td className="px-4 py-3">
                  <span
                    className={`px-2 py-0.5 rounded text-xs ${
                      m.is_available
                        ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
                        : 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400'
                    }`}
                  >
                    {m.is_available ? '可用' : '下线'}
                  </span>
                </td>
                <td className="px-4 py-3">
                  <button
                    onClick={() => {
                      if (confirm(`确定删除模型 "${m.model_name}"？关联别名将被标记失效。`))
                        remove.mutate(m.id);
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

      <AliasPanel />
    </div>
  );
}
