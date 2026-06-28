import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { settingsApi } from '../lib/api';

export function SettingsPage() {
  const queryClient = useQueryClient();
  const { data, isLoading, error } = useQuery({
    queryKey: ['auto-refresh'],
    queryFn: settingsApi.getAutoRefresh,
  });

  const toggle = useMutation({
    mutationFn: (enabled: boolean) => settingsApi.setAutoRefresh(enabled),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['auto-refresh'] }),
    onError: (e: Error) => alert(`切换失败: ${e.message}`),
  });

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">设置</h1>
        <p className="text-sm text-gray-500 mt-1">应用配置与模型刷新策略</p>
      </div>

      {isLoading && <p className="text-gray-500">加载中...</p>}
      {error && <p className="text-red-500">加载失败: {error.message}</p>}

      {data && (
        <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-5 space-y-4">
          <div>
            <h2 className="font-semibold">模型自动刷新</h2>
            <p className="text-xs text-gray-500 mt-0.5">
              开启后：应用启动时刷新一次上游模型，之后每 6 小时（±随机 30 分钟）自动刷新。关闭时仅手动刷新。
            </p>
          </div>

          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium">自动刷新</p>
              <p className="text-xs text-gray-500">
                {data.enabled ? '已开启' : '已关闭（默认）'}
              </p>
            </div>
            <button
              onClick={() => toggle.mutate(!data.enabled)}
              disabled={toggle.isPending}
              className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                data.enabled ? 'bg-green-600' : 'bg-gray-300 dark:bg-gray-700'
              }`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                  data.enabled ? 'translate-x-6' : 'translate-x-1'
                }`}
              />
            </button>
          </div>

          <div className="border-t border-gray-200 dark:border-gray-800 pt-4 space-y-2 text-sm">
            <div className="flex justify-between">
              <span className="text-gray-500">最近同步时间</span>
              <span className="font-mono text-xs">{data.last_sync_at ?? '从未同步'}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-500">最近同步错误</span>
              <span
                className={`text-xs max-w-md text-right ${
                  data.last_sync_error ? 'text-red-500' : 'text-gray-400'
                }`}
              >
                {data.last_sync_error ?? '无'}
              </span>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}
