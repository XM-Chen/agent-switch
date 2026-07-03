import { useQuery } from '@tanstack/react-query';
import { logsApi, type LogType } from '../lib/api';
import { formatTime } from '../lib/format';
import { useState, useMemo } from 'react';

/** 工具选项。 */
const TOOL_OPTIONS = [
  { value: '', label: '全部' },
  { value: 'claude-code', label: 'Claude Code' },
  { value: 'codex', label: 'Codex' },
];

/** 日志类型过滤选项。 */
const LOG_TYPE_OPTIONS = [
  { value: '', label: '全部' },
  { value: 'production', label: '生产' },
  { value: 'test', label: '测试' },
];

export function LogsPage() {
  const [tool, setTool] = useState('');
  const [logType, setLogType] = useState('');
  const [status, setStatus] = useState('');
  const [limit, setLimit] = useState(50);
  const [offset, setOffset] = useState(0);
  const [selectedId, setSelectedId] = useState<string | null>(null);

  // 日志列表查询
  const params = useMemo(
    () => ({
      tool: tool || undefined,
      log_type: logType ? (logType as LogType) : undefined,
      status: status ? Number(status) : undefined,
      limit,
      offset,
    }),
    [tool, logType, status, limit, offset],
  );

  const { data, isLoading, error } = useQuery({
    queryKey: ['logs', params],
    queryFn: () => logsApi.list(params),
  });

  // 选中详情
  const { data: detail } = useQuery({
    queryKey: ['logs', selectedId],
    queryFn: () => logsApi.get(selectedId!),
    enabled: !!selectedId,
  });

  const items = data?.items ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.ceil(total / limit);
  const currentPage = Math.floor(offset / limit) + 1;

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">请求日志</h1>
        <p className="text-sm text-gray-500 mt-1">
          代理转发请求的摘要与链路轨迹
        </p>
      </div>

      {/* 过滤栏 */}
      <div className="flex flex-wrap gap-3 items-end">
        <div>
          <label className="block text-xs text-gray-500 mb-1">工具</label>
          <select
            value={tool}
            onChange={(e) => { setTool(e.target.value); setOffset(0); }}
            className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          >
            {TOOL_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>{opt.label}</option>
            ))}
          </select>
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">类型</label>
          <select
            value={logType}
            onChange={(e) => { setLogType(e.target.value); setOffset(0); }}
            className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          >
            {LOG_TYPE_OPTIONS.map((opt) => (
              <option key={opt.value} value={opt.value}>{opt.label}</option>
            ))}
          </select>
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">状态码</label>
          <input
            type="number"
            value={status}
            onChange={(e) => { setStatus(e.target.value); setOffset(0); }}
            placeholder="例如 200"
            className="w-24 px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">每页</label>
          <select
            value={limit}
            onChange={(e) => { setLimit(Number(e.target.value)); setOffset(0); }}
            className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
          >
            <option value={20}>20</option>
            <option value={50}>50</option>
            <option value={100}>100</option>
          </select>
        </div>
      </div>

      {/* 主体区域 */}
      <div className="grid grid-cols-1 xl:grid-cols-2 gap-6">
        {/* 日志列表 */}
        <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 overflow-hidden">
          {isLoading && <p className="p-4 text-gray-500">加载中...</p>}
          {error && <p className="p-4 text-red-500">加载失败: {error.message}</p>}

          {!isLoading && items.length === 0 && (
            <p className="p-8 text-center text-gray-400">暂无日志记录</p>
          )}

          {items.length > 0 && (
            <div className="divide-y divide-gray-100 dark:divide-gray-800 max-h-[600px] overflow-y-auto">
              {items.map((entry) => (
                <button
                  key={entry.id}
                  onClick={() => setSelectedId(selectedId === entry.id ? null : entry.id)}
                  className={`w-full text-left px-4 py-3 hover:bg-gray-50 dark:hover:bg-gray-800/50 transition-colors ${
                    selectedId === entry.id ? 'bg-blue-50 dark:bg-blue-900/10' : ''
                  }`}
                >
                  <div className="flex items-center justify-between mb-1">
                    <span className="text-xs font-mono text-gray-400">
                      {entry.created_at ? formatTime(entry.created_at) : '—'}
                    </span>
                    <StatusBadge status={entry.status} />
                  </div>
                  <p className="text-sm font-medium truncate">
                    {entry.tool ?? '—'} / {entry.inbound_endpoint}
                  </p>
                  <div className="flex items-center gap-2 mt-1 text-xs text-gray-500">
                    {entry.requested_model && <span>{entry.requested_model}</span>}
                    {entry.upstream_model && (
                      <>
                        <span>&rarr;</span>
                        <span className="font-mono">{entry.upstream_model}</span>
                      </>
                    )}
                    {entry.duration_ms != null && (
                      <>
                        <span className="text-gray-300">|</span>
                        <span>{entry.duration_ms}ms</span>
                      </>
                    )}
                  </div>
                </button>
              ))}
            </div>
          )}

          {/* 分页 */}
          {total > limit && (
            <div className="flex items-center justify-between px-4 py-3 border-t border-gray-100 dark:border-gray-800 text-sm">
              <span className="text-gray-500">
                共 {total} 条，第 {currentPage}/{totalPages} 页
              </span>
              <div className="flex gap-2">
                <button
                  onClick={() => setOffset(Math.max(0, offset - limit))}
                  disabled={offset === 0}
                  className="px-3 py-1 rounded border border-gray-300 dark:border-gray-700 disabled:opacity-40 hover:bg-gray-50 dark:hover:bg-gray-800"
                >
                  上一页
                </button>
                <button
                  onClick={() => setOffset(offset + limit)}
                  disabled={offset + limit >= total}
                  className="px-3 py-1 rounded border border-gray-300 dark:border-gray-700 disabled:opacity-40 hover:bg-gray-50 dark:hover:bg-gray-800"
                >
                  下一页
                </button>
              </div>
            </div>
          )}
        </div>

        {/* 详情面板 */}
        <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 overflow-hidden">
          {!detail ? (
            <p className="p-8 text-center text-gray-400">
              {selectedId ? '加载中...' : '点击左侧日志条目查看详情'}
            </p>
          ) : (
            <div className="p-4 space-y-4 overflow-y-auto max-h-[600px]">
              <h3 className="font-semibold">请求详情</h3>

              {/* 基本信息 */}
              <Section title="基本信息">
                <Field label="日志 ID" value={detail.id} mono />
                <Field label="请求 ID" value={detail.request_id} mono />
                <Field label="工具" value={detail.tool ?? '—'} />
                <Field label="入站地址" value={detail.inbound_endpoint} mono />
                <Field label="状态码" value={detail.status != null ? String(detail.status) : '—'} />
                {detail.error_kind && <Field label="错误类型" value={detail.error_kind} />}
              </Section>

              {/* 模型映射 */}
              <Section title="模型映射">
                <Field label="请求模型" value={detail.requested_model ?? '—'} />
                <Field label="解析别名" value={detail.resolved_alias ?? '—'} />
                <Field label="解析作用域" value={detail.resolved_scope ?? '—'} />
                <Field label="目标端点" value={detail.target_endpoint_id ?? '—'} mono />
                <Field label="上游模型" value={detail.upstream_model ?? '—'} />
              </Section>

              {/* 协议转换 */}
              <Section title="协议转换">
                <Field label="上游端点" value={detail.upstream_endpoint ?? '—'} mono />
                <Field label="协议来源" value={detail.protocol_from ?? '—'} />
                <Field label="协议目标" value={detail.protocol_to ?? '—'} />
              </Section>

              {/* 性能统计 */}
              <Section title="性能统计">
                <Field label="是否流式" value={detail.stream ? '是' : '否'} />
                <Field label="总耗时" value={detail.duration_ms != null ? `${detail.duration_ms}ms` : '—'} />
                <Field label="首 Token 耗时" value={detail.first_token_ms != null ? `${detail.first_token_ms}ms` : '—'} />
                <Field label="输入 Tokens" value={detail.input_tokens != null ? String(detail.input_tokens) : '—'} />
                <Field label="输出 Tokens" value={detail.output_tokens != null ? String(detail.output_tokens) : '—'} />
                <Field label="缓存创建" value={detail.cache_creation_tokens != null ? String(detail.cache_creation_tokens) : '—'} />
                <Field label="缓存读取" value={detail.cache_read_tokens != null ? String(detail.cache_read_tokens) : '—'} />
              </Section>

              {/* 故障转移链路 */}
              <Section title="故障转移链路">
                {detail.fallback_chain ? (
                  <FallbackChainView chain={detail.fallback_chain} />
                ) : (
                  <p className="text-sm text-gray-400">无故障转移记录</p>
                )}
              </Section>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

// ── 辅助组件 ────────────────────────────────────────────

/** 状态标记。 */
function StatusBadge({ status }: { status: number | null }) {
  if (status == null) return <span className="text-xs text-gray-400">—</span>;
  if (status >= 200 && status < 300) {
    return <span className="px-1.5 py-0.5 rounded text-xs bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400">{status}</span>;
  }
  if (status >= 500) {
    return <span className="px-1.5 py-0.5 rounded text-xs bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400">{status}</span>;
  }
  return <span className="px-1.5 py-0.5 rounded text-xs bg-yellow-100 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400">{status}</span>;
}

/** 分节标题。 */
function Section({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <div>
      <h4 className="text-xs font-semibold text-gray-500 uppercase tracking-wide mb-2">{title}</h4>
      <div className="space-y-1.5">{children}</div>
    </div>
  );
}

/** 键值对字段。 */
function Field({ label, value, mono }: { label: string; value: string; mono?: boolean }) {
  return (
    <div className="flex items-start gap-2 text-sm">
      <span className="text-gray-500 w-28 shrink-0">{label}</span>
      <span className={`${mono ? 'font-mono text-xs' : ''} text-gray-900 dark:text-gray-100 break-all`}>
        {value}
      </span>
    </div>
  );
}

/** 故障转移链路可视化。 */
function FallbackChainView({ chain }: { chain: string }) {
  let hops: { endpoint_id: string; model: string; status: number; error: string }[];
  try {
    hops = JSON.parse(chain);
  } catch {
    return <p className="text-sm text-red-500">无法解析 fallback 链路: {chain}</p>;
  }

  if (!Array.isArray(hops) || hops.length === 0) {
    return <p className="text-sm text-gray-400">无故障转移记录</p>;
  }

  return (
    <div className="space-y-2">
      {hops.map((hop, idx) => (
        <div key={idx} className="flex items-start gap-2 text-sm">
          <span className="text-gray-400 font-mono text-xs mt-0.5 w-5 shrink-0">#{idx + 1}</span>
          <div className="flex-1 min-w-0">
            <div className="flex items-center gap-2">
              <span className="font-mono text-xs truncate">{hop.endpoint_id}</span>
              <StatusBadge status={hop.status} />
            </div>
            {hop.model && <p className="text-xs text-gray-500">模型: {hop.model}</p>}
            {hop.error && <p className="text-xs text-red-500">{hop.error}</p>}
          </div>
          {idx < hops.length - 1 && (
            <span className="text-gray-300 dark:text-gray-600 mt-1">&darr;</span>
          )}
        </div>
      ))}
    </div>
  );
}
