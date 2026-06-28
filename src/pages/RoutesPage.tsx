import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { routesApi, type RouteSettings } from '../lib/api';
import { useState } from 'react';

/** 策略选项。 */
const STRATEGY_OPTIONS = [
  { value: 'fill-first', label: 'Fill-First（按优先级）' },
  { value: 'round-robin', label: 'Round-Robin（轮询）' },
];

export function RoutesPage() {
  const { data: routes = [], isLoading, error } = useQuery({
    queryKey: ['routes'],
    queryFn: routesApi.list,
  });

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">路由</h1>
        <p className="text-sm text-gray-500 mt-1">
          代理转发路由策略与故障转移配置，管理候选端点状态
        </p>
      </div>

      {isLoading && <p className="text-gray-500">加载中...</p>}
      {error && <p className="text-red-500">加载失败: {error.message}</p>}

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {routes.map((route) => (
          <RouteCard key={route.id} route={route} />
        ))}
      </div>

      {!isLoading && routes.length === 0 && (
        <p className="text-gray-400 text-center py-10">
          暂未配置路由。请先完成数据库迁移。
        </p>
      )}
    </div>
  );
}

/** 单条路由卡片。 */
function RouteCard({ route }: { route: RouteSettings }) {
  const [editing, setEditing] = useState(false);

  return (
    <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 overflow-hidden">
      {/* 卡片标题 */}
      <div className="flex items-center justify-between px-4 py-3 border-b border-gray-100 dark:border-gray-800">
        <div>
          <h2 className="font-semibold text-lg">{route.label}</h2>
          <p className="text-xs text-gray-500 font-mono">{route.id}</p>
        </div>
        <span className="px-2 py-0.5 rounded text-xs bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400">
          {route.protocol_type}
        </span>
      </div>

      {/* 设置区域 */}
      <div className="p-4 space-y-4">
        {editing ? (
          <RouteSettingsForm route={route} onSaved={() => setEditing(false)} />
        ) : (
          <>
            <div className="grid grid-cols-2 gap-3 text-sm">
              <div>
                <span className="text-gray-500">策略</span>
                <p className="font-medium">{route.strategy === 'fill-first' ? 'Fill-First' : 'Round-Robin'}</p>
              </div>
              <div>
                <span className="text-gray-500">故障转移</span>
                <p className="font-medium">{route.failover_enabled ? '已启用' : '已禁用'}</p>
              </div>
              <div>
                <span className="text-gray-500">最大切换</span>
                <p className="font-medium">{route.max_switches}</p>
              </div>
              <div>
                <span className="text-gray-500">同端点重试</span>
                <p className="font-medium">{route.same_account_retries}</p>
              </div>
              <div>
                <span className="text-gray-500">冷却系数</span>
                <p className="font-medium">{route.cooldown_multiplier}x</p>
              </div>
            </div>
            <button
              onClick={() => setEditing(true)}
              className="text-sm text-blue-600 hover:text-blue-700"
            >
              编辑设置
            </button>
          </>
        )}
      </div>

      {/* 候选端点列表 */}
      <div className="border-t border-gray-100 dark:border-gray-800">
        <div className="px-4 py-2 bg-gray-50 dark:bg-gray-800/50 text-xs text-gray-500 font-medium">
          候选端点（{route.candidates.length}）
        </div>
        {route.candidates.length === 0 ? (
          <p className="px-4 py-3 text-sm text-gray-400">
            暂无匹配此协议的端点
          </p>
        ) : (
          <div className="divide-y divide-gray-100 dark:divide-gray-800">
            {route.candidates.map((c) => (
              <div key={c.id} className="px-4 py-2.5 flex items-center justify-between text-sm">
                <div className="flex-1 min-w-0">
                  <p className="font-medium truncate">{c.name}</p>
                  <p className="text-xs text-gray-500 font-mono truncate">{c.base_url}</p>
                </div>
                <div className="flex items-center gap-2 ml-3">
                  <span className="text-xs text-gray-400">P{c.priority}</span>
                  <EndpointBadge candidate={c} />
                </div>
              </div>
            ))}
          </div>
        )}
      </div>
    </div>
  );
}

/** 端点健康状态标记。 */
function EndpointBadge({ candidate }: { candidate: RouteSettings['candidates'][0] }) {
  if (!candidate.enabled) {
    return <span className="px-2 py-0.5 rounded text-xs bg-gray-100 text-gray-500 dark:bg-gray-800">已禁用</span>;
  }
  if (candidate.cooldown_until && new Date(candidate.cooldown_until) > new Date()) {
    return <span className="px-2 py-0.5 rounded text-xs bg-yellow-100 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400">冷却中</span>;
  }
  if (candidate.last_error_kind) {
    return <span className="px-2 py-0.5 rounded text-xs bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400">异常</span>;
  }
  if (candidate.last_success_at) {
    return <span className="px-2 py-0.5 rounded text-xs bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400">正常</span>;
  }
  return <span className="px-2 py-0.5 rounded text-xs bg-gray-100 text-gray-500 dark:bg-gray-800">待用</span>;
}

/** 路由设置编辑表单。 */
function RouteSettingsForm({ route, onSaved }: { route: RouteSettings; onSaved: () => void }) {
  const queryClient = useQueryClient();
  const [strategy, setStrategy] = useState(route.strategy);
  const [failoverEnabled, setFailoverEnabled] = useState(route.failover_enabled);
  const [maxSwitches, setMaxSwitches] = useState(route.max_switches);
  const [sameAccountRetries, setSameAccountRetries] = useState(route.same_account_retries);
  const [cooldownMultiplier, setCooldownMultiplier] = useState(route.cooldown_multiplier);

  const update = useMutation({
    mutationFn: () =>
      routesApi.update(route.id, {
        strategy,
        failover_enabled: failoverEnabled,
        max_switches: maxSwitches,
        same_account_retries: sameAccountRetries,
        cooldown_multiplier: cooldownMultiplier,
      }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ['routes'] });
      onSaved();
    },
    onError: (e: Error) => alert(`更新失败: ${e.message}`),
  });

  return (
    <div className="space-y-3">
      <div>
        <label className="block text-xs text-gray-500 mb-1">选择策略</label>
        <select
          value={strategy}
          onChange={(e) => setStrategy(e.target.value)}
          className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
        >
          {STRATEGY_OPTIONS.map((opt) => (
            <option key={opt.value} value={opt.value}>
              {opt.label}
            </option>
          ))}
        </select>
      </div>

      <div className="flex items-center gap-2">
        <input
          type="checkbox"
          id="failover"
          checked={failoverEnabled}
          onChange={(e) => setFailoverEnabled(e.target.checked)}
          className="rounded"
        />
        <label htmlFor="failover" className="text-sm">启用故障转移</label>
      </div>

      <div className="grid grid-cols-2 gap-3">
        <div>
          <label className="block text-xs text-gray-500 mb-1">最大切换次数</label>
          <input
            type="number"
            value={maxSwitches}
            onChange={(e) => setMaxSwitches(Number(e.target.value))}
            min={1}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">同端点重试次数</label>
          <input
            type="number"
            value={sameAccountRetries}
            onChange={(e) => setSameAccountRetries(Number(e.target.value))}
            min={0}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          />
        </div>
      </div>

      <div>
        <label className="block text-xs text-gray-500 mb-1">冷却系数</label>
        <input
          type="number"
          value={cooldownMultiplier}
          onChange={(e) => setCooldownMultiplier(Number(e.target.value))}
          min={0.1}
          step={0.1}
          className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
        />
      </div>

      <div className="flex gap-2 pt-1">
        <button
          onClick={() => update.mutate()}
          disabled={update.isPending}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
        >
          {update.isPending ? '保存中...' : '保存'}
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
