import { useQuery } from '@tanstack/react-query';
import { useNavigate } from 'react-router-dom';
import {
  accountsApi,
  endpointsApi,
  modelsApi,
  routesApi,
  toolsApi,
  logsApi,
  settingsApi,
  type ToolStatus,
  type LogEntry,
} from '../lib/api';

// ── 标签映射（与 ToolCard.tsx 一致，避免引入跨组件依赖） ─────

const TOOL_LABELS: Record<string, string> = {
  'claude-code': 'Claude Code',
  codex: 'Codex',
  opencode: 'OpenCode',
};

const CATEGORY_LABELS: Record<string, string> = {
  agent_switch: 'agent-switch',
  official: '官方',
  third_party: '第三方',
  unconfigured: '未配置',
  unrecognized: '无法识别',
};

const CATEGORY_COLORS: Record<string, string> = {
  agent_switch: 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400',
  official: 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400',
  third_party: 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400',
  unconfigured: 'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400',
  unrecognized: 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400',
};

export function DashboardPage() {
  const navigate = useNavigate();

  // D1：纯前端组合 7 个 TanStack Query，queryKey 按资源命名（R7.1）
  const { data: accounts = [], isLoading: accountsLoading } = useQuery({
    queryKey: ['accounts'],
    queryFn: accountsApi.list,
  });
  const { data: endpoints = [], isLoading: endpointsLoading } = useQuery({
    queryKey: ['endpoints'],
    queryFn: endpointsApi.list,
  });
  const { data: models = [], isLoading: modelsLoading } = useQuery({
    queryKey: ['models'],
    queryFn: () => modelsApi.list(),
  });
  const { data: routes = [], isLoading: routesLoading } = useQuery({
    queryKey: ['routes'],
    queryFn: routesApi.list,
  });
  const { data: tools = [], isLoading: toolsLoading } = useQuery({
    queryKey: ['tools'],
    queryFn: toolsApi.list,
  });
  const { data: logsResp, isLoading: logsLoading } = useQuery({
    queryKey: ['logs'],
    queryFn: () => logsApi.list({ limit: 10 }),
  });
  const { data: autoRefresh, isLoading: autoRefreshLoading } = useQuery({
    queryKey: ['auto-refresh'],
    queryFn: settingsApi.getAutoRefresh,
  });

  const logs = logsResp?.items ?? [];

  // 端点分桶（R5.1）：启用·禁用 + 健康
  const enabledCount = endpoints.filter((e) => e.enabled).length;
  const disabledCount = endpoints.length - enabledCount;

  // 模型分桶：custom + synced（R1.1）
  const customModels = models.filter((m) => m.source === 'custom').length;
  const syncedModels = models.filter((m) => m.source === 'synced').length;

  // 路由：failover_enabled 数（D3）
  const failoverRoutes = routes.filter((r) => r.failover_enabled).length;

  // 端点健康聚合（D3 / R5）：聚合各路由 candidates
  const health = aggregateEndpointHealth(endpoints, routes);
  const hasAbnormalEndpoint = health.cooling > 0 || health.recentFailure > 0;

  // 首次无数据引导（R6.1）：账号/端点/模型/路由全空
  const totalResources =
    accounts.length + endpoints.length + models.length + routes.length;
  const allLoaded =
    !accountsLoading &&
    !endpointsLoading &&
    !modelsLoading &&
    !routesLoading;

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">总览</h1>
        <p className="text-sm text-gray-500 mt-1">
          账号、端点、模型与路由的全局概览
        </p>
      </div>

      {/* R6.1 无数据引导（不阻塞主界面：仅在全空时顶部提示） */}
      {allLoaded && totalResources === 0 && (
        <EmptyGuide navigate={navigate} />
      )}

      {/* R1 / D2 响应式网格统计卡 */}
      <div className="grid grid-cols-2 lg:grid-cols-4 gap-4">
        <CountCard
          title="账号"
          value={accounts.length}
          loading={accountsLoading}
          sub="上游供应商账号"
          onClick={() => navigate('/accounts')}
        />
        <CountCard
          title="端点"
          value={endpoints.length}
          loading={endpointsLoading}
          sub={`启用 ${enabledCount} · 禁用 ${disabledCount}`}
          onClick={() => navigate('/endpoints')}
        />
        <CountCard
          title="模型"
          value={models.length}
          loading={modelsLoading}
          sub={`自定义 ${customModels} · 同步 ${syncedModels}`}
          onClick={() => navigate('/models')}
        />
        <CountCard
          title="路由"
          value={routes.length}
          loading={routesLoading}
          sub={`故障转移 ${failoverRoutes}`}
          onClick={() => navigate('/routes')}
        />
      </div>

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-6">
        {/* R2 工具接管状态 */}
        <SectionCard
          title="工具接管状态"
          loading={toolsLoading}
          onTitleClick={() => navigate('/tools')}
        >
          {tools.length === 0 ? (
            <EmptyRow text="暂无工具状态" />
          ) : (
            <div className="divide-y divide-gray-100 dark:divide-gray-800">
              {tools.map((tool) => (
                <ToolRow key={tool.tool} tool={tool} />
              ))}
            </div>
          )}
        </SectionCard>

        {/* R3 模型自动刷新状态 */}
        <SectionCard
          title="模型自动刷新"
          loading={autoRefreshLoading}
          onTitleClick={() => navigate('/settings')}
        >
          {!autoRefresh ? (
            <EmptyRow text="暂无自动刷新状态" />
          ) : (
            <div className="space-y-2 text-sm">
              <div className="flex items-center justify-between">
                <span className="text-gray-500">自动刷新</span>
                <span
                  className={
                    autoRefresh.enabled
                      ? 'text-green-600 font-medium'
                      : 'text-gray-400'
                  }
                >
                  {autoRefresh.enabled ? '已开启' : '已关闭'}
                </span>
              </div>
              <div className="flex items-center justify-between">
                <span className="text-gray-500">最近同步时间</span>
                <span className="font-mono text-xs text-gray-600 dark:text-gray-400">
                  {autoRefresh.last_sync_at ?? '从未同步'}
                </span>
              </div>
              {autoRefresh.last_sync_error && (
                <div className="flex items-start justify-between gap-3">
                  <span className="text-gray-500 shrink-0">最近同步错误</span>
                  <span className="text-xs text-red-500 text-right max-w-md break-all">
                    {autoRefresh.last_sync_error}
                  </span>
                </div>
              )}
              <button
                onClick={() => navigate('/settings')}
                className="text-xs text-blue-600 hover:text-blue-700 pt-1"
              >
                前往设置 &rarr;
              </button>
            </div>
          )}
        </SectionCard>

        {/* R5 端点健康 */}
        <SectionCard
          title="端点健康"
          loading={endpointsLoading || routesLoading}
          onTitleClick={() => navigate('/endpoints')}
        >
          <div className="grid grid-cols-2 gap-3 text-sm">
            <HealthStat label="正常" value={health.normal} tone="green" />
            <HealthStat label="冷却中" value={health.cooling} tone="yellow" />
            <HealthStat
              label="最近失败"
              value={health.recentFailure}
              tone="red"
            />
            <HealthStat label="待用" value={health.idle} tone="gray" />
          </div>
          <div className="mt-3 pt-3 border-t border-gray-100 dark:border-gray-800 flex items-center justify-between text-sm">
            <span className="text-gray-500">
              启用 {enabledCount} · 禁用 {disabledCount}
            </span>
            {hasAbnormalEndpoint ? (
              <button
                onClick={() => navigate('/endpoints')}
                className="text-xs text-red-600 hover:text-red-700"
              >
                存在异常端点，前往查看 &rarr;
              </button>
            ) : (
              <button
                onClick={() => navigate('/endpoints')}
                className="text-xs text-blue-600 hover:text-blue-700"
              >
                查看端点 &rarr;
              </button>
            )}
          </div>
        </SectionCard>

        {/* R4 近期请求日志（10 条） */}
        <SectionCard
          title="近期请求日志"
          loading={logsLoading}
          onTitleClick={() => navigate('/logs')}
        >
          {logs.length === 0 ? (
            <EmptyRow text="暂无请求日志" />
          ) : (
            <div className="divide-y divide-gray-100 dark:divide-gray-800 max-h-72 overflow-y-auto">
              {logs.map((entry) => (
                <LogRow key={entry.id} entry={entry} />
              ))}
            </div>
          )}
          <div className="pt-2 text-right">
            <button
              onClick={() => navigate('/logs')}
              className="text-xs text-blue-600 hover:text-blue-700"
            >
              查看全部日志 &rarr;
            </button>
          </div>
        </SectionCard>
      </div>
    </div>
  );
}

// ── 计数卡 ──────────────────────────────────────────────

interface CountCardProps {
  title: string;
  value: number;
  loading: boolean;
  sub: string;
  onClick: () => void;
}

function CountCard({ title, value, loading, sub, onClick }: CountCardProps) {
  return (
    <button
      onClick={onClick}
      className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-5 text-left hover:border-blue-400 dark:hover:border-blue-600 transition-colors"
    >
      <p className="text-xs text-gray-500">{title}</p>
      {loading ? (
        <div className="mt-2 h-8 w-12 animate-pulse rounded bg-gray-100 dark:bg-gray-800" />
      ) : (
        <p className="mt-1 text-3xl font-bold">{value}</p>
      )}
      <p className="mt-1 text-xs text-gray-400">{sub}</p>
    </button>
  );
}

// ── 区块卡片（标题可点击跳转） ──────────────────────────

interface SectionCardProps {
  title: string;
  loading: boolean;
  onTitleClick: () => void;
  children: React.ReactNode;
}

function SectionCard({
  title,
  loading,
  onTitleClick,
  children,
}: SectionCardProps) {
  return (
    <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 overflow-hidden">
      <div className="flex items-center justify-between px-4 py-3 border-b border-gray-100 dark:border-gray-800">
        <h2 className="font-semibold">{title}</h2>
        <button
          onClick={onTitleClick}
          className="text-xs text-blue-600 hover:text-blue-700"
        >
          管理 &rarr;
        </button>
      </div>
      <div className="p-4">
        {loading ? (
          <div className="space-y-2">
            <div className="h-4 w-full animate-pulse rounded bg-gray-100 dark:bg-gray-800" />
            <div className="h-4 w-2/3 animate-pulse rounded bg-gray-100 dark:bg-gray-800" />
          </div>
        ) : (
          children
        )}
      </div>
    </div>
  );
}

// ── 工具接管行 ──────────────────────────────────────────

function ToolRow({ tool }: { tool: ToolStatus }) {
  return (
    <div className="flex items-center justify-between py-2.5 text-sm">
      <div className="flex items-center gap-2 min-w-0">
        <span className="font-medium">
          {TOOL_LABELS[tool.tool] || tool.tool}
        </span>
        <span
          className={`px-2 py-0.5 rounded text-xs font-medium ${
            CATEGORY_COLORS[tool.live_category] || CATEGORY_COLORS.unrecognized
          }`}
        >
          {CATEGORY_LABELS[tool.live_category] || tool.live_category}
        </span>
      </div>
      <span className="shrink-0">
        {!tool.supports_takeover ? (
          <span className="px-2 py-0.5 rounded text-xs bg-gray-100 text-gray-500 dark:bg-gray-800">
            仅手动配置
          </span>
        ) : (
          <span
            className={
              tool.enabled
                ? 'px-2 py-0.5 rounded text-xs bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
                : 'px-2 py-0.5 rounded text-xs bg-gray-100 text-gray-500 dark:bg-gray-800'
            }
          >
            {tool.enabled ? '已接管' : '未接管'}
          </span>
        )}
      </span>
    </div>
  );
}

// ── 健康分桶小卡 ────────────────────────────────────────

const HEALTH_TONES: Record<string, string> = {
  green: 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400',
  yellow: 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400',
  red: 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400',
  gray: 'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400',
};

function HealthStat({
  label,
  value,
  tone,
}: {
  label: string;
  value: number;
  tone: keyof typeof HEALTH_TONES;
}) {
  return (
    <div className="flex items-center justify-between rounded-md border border-gray-100 dark:border-gray-800 px-3 py-2">
      <span className="text-gray-500">{label}</span>
      <span className={`px-2 py-0.5 rounded text-xs font-medium ${HEALTH_TONES[tone]}`}>
        {value}
      </span>
    </div>
  );
}

// ── 日志行 ──────────────────────────────────────────────

function LogRow({ entry }: { entry: LogEntry }) {
  const failed =
    entry.error_kind != null ||
    entry.status == null ||
    entry.status < 200 ||
    entry.status >= 300;
  const hops = countFallbackHops(entry.fallback_chain);

  return (
    <div className="w-full text-left py-2.5 text-sm">
      <div className="flex items-center justify-between mb-0.5">
        <span className="text-xs font-mono text-gray-400">
          {entry.created_at ? formatTime(entry.created_at) : '—'}
        </span>
        <div className="flex items-center gap-2">
          {hops > 1 && (
            <span className="text-xs text-orange-500">跳 {hops}</span>
          )}
          <StatusBadge status={entry.status} failed={failed} />
        </div>
      </div>
      <div className="flex items-center gap-2 text-xs text-gray-500">
        <span className="truncate">{entry.tool ?? '—'}</span>
        {entry.duration_ms != null && (
          <span className="text-gray-300">|</span>
        )}
        {entry.duration_ms != null && <span>{entry.duration_ms}ms</span>}
      </div>
    </div>
  );
}

// ── 状态标记 ────────────────────────────────────────────

function StatusBadge({
  status,
  failed,
}: {
  status: number | null;
  failed: boolean;
}) {
  if (status == null) {
    return (
      <span className="px-1.5 py-0.5 rounded text-xs bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400">
        失败
      </span>
    );
  }
  if (failed) {
    return (
      <span className="px-1.5 py-0.5 rounded text-xs bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400">
        {status}
      </span>
    );
  }
  return (
    <span className="px-1.5 py-0.5 rounded text-xs bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400">
      {status}
    </span>
  );
}

// ── 空行占位 ────────────────────────────────────────────

function EmptyRow({ text }: { text: string }) {
  return <p className="text-sm text-gray-400 text-center py-4">{text}</p>;
}

// ── 全空引导（R6.1） ────────────────────────────────────

function EmptyGuide({ navigate }: { navigate: (path: string) => void }) {
  return (
    <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-6 text-center space-y-3">
      <p className="text-gray-600 dark:text-gray-300">欢迎使用 Agent-Switch</p>
      <p className="text-sm text-gray-500">
        尚未添加任何账号、端点、模型或路由。请先添加上游账号开始使用。
      </p>
      <div className="flex items-center justify-center gap-2 pt-1">
        <button
          onClick={() => navigate('/accounts')}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700"
        >
          前往账号页
        </button>
        <button
          onClick={() => navigate('/endpoints')}
          className="px-4 py-2 bg-gray-100 dark:bg-gray-800 rounded-md text-sm hover:bg-gray-200 dark:hover:bg-gray-700"
        >
          前往端点页
        </button>
      </div>
    </div>
  );
}

// ── 辅助函数 ────────────────────────────────────────────

/** 端点健康聚合（D3 / R5）：正常 / 冷却中 / 最近失败 / 待用。 */
function aggregateEndpointHealth(
  endpoints: { enabled: boolean; cooldown_until: string | null; last_failure_at: string | null; last_success_at: string | null }[],
  routes: { candidates: { cooldown_until: string | null; last_failure_at: string | null; last_success_at: string | null }[] }[],
) {
  const now = Date.now();
  // 最近失败窗口：1 小时内视为"最近失败"
  const recentFailureWindow = 60 * 60 * 1000;

  // 优先用 endpoints 列表（含启用·禁用）；routes candidates 作为补充
  let normal = 0;
  let cooling = 0;
  let recentFailure = 0;
  let idle = 0;

  for (const e of endpoints) {
    const bucket = bucketHealth(
      e.cooldown_until,
      e.last_failure_at,
      e.last_success_at,
      now,
      recentFailureWindow,
    );
    if (bucket === 'normal') normal++;
    else if (bucket === 'cooling') cooling++;
    else if (bucket === 'recent_failure') recentFailure++;
    else idle++;
  }

  // endpoints 为空但 routes 有候选时，用 candidates 兜底聚合
  if (endpoints.length === 0) {
    for (const route of routes) {
      for (const c of route.candidates) {
        const bucket = bucketHealth(
          c.cooldown_until,
          c.last_failure_at,
          c.last_success_at,
          now,
          recentFailureWindow,
        );
        if (bucket === 'normal') normal++;
        else if (bucket === 'cooling') cooling++;
        else if (bucket === 'recent_failure') recentFailure++;
        else idle++;
      }
    }
  }

  return { normal, cooling, recentFailure, idle };
}

function bucketHealth(
  cooldownUntil: string | null,
  lastFailureAt: string | null,
  lastSuccessAt: string | null,
  now: number,
  recentFailureWindow: number,
): 'normal' | 'cooling' | 'recent_failure' | 'idle' {
  if (cooldownUntil) {
    const cd = Date.parse(cooldownUntil);
    if (!Number.isNaN(cd) && cd > now) return 'cooling';
  }
  if (lastFailureAt) {
    const lf = Date.parse(lastFailureAt);
    if (!Number.isNaN(lf) && now - lf <= recentFailureWindow) {
      return 'recent_failure';
    }
  }
  if (lastSuccessAt) {
    const ls = Date.parse(lastSuccessAt);
    if (!Number.isNaN(ls)) return 'normal';
  }
  return 'idle';
}

/** 解析 fallback_chain（JSON 数组）取跳数；失败或空返回 0。 */
function countFallbackHops(chain: string | null): number {
  if (!chain) return 0;
  try {
    const parsed = JSON.parse(chain);
    if (Array.isArray(parsed)) return parsed.length;
  } catch {
    // 解析失败忽略
  }
  return 0;
}

/** 格式化 ISO 时间为短格式 HH:mm:ss（与 LogsPage 一致）。 */
function formatTime(iso: string): string {
  try {
    const d = new Date(iso);
    const hh = String(d.getHours()).padStart(2, '0');
    const mm = String(d.getMinutes()).padStart(2, '0');
    const ss = String(d.getSeconds()).padStart(2, '0');
    return `${hh}:${mm}:${ss}`;
  } catch {
    return iso;
  }
}
