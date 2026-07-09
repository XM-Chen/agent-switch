import { useQuery } from '@tanstack/react-query';
import { useMemo, useState } from 'react';
import { sessionsApi, type SessionMessage, type SessionMeta } from '../lib/api';

const DEFAULT_LIMIT = 50;
const LONG_CONTENT_LENGTH = 1200;

export function SessionsPage() {
  const [search, setSearch] = useState('');
  const [limit, setLimit] = useState(DEFAULT_LIMIT);
  const [offset, setOffset] = useState(0);
  const [selected, setSelected] = useState<SessionMeta | null>(null);

  const params = useMemo(
    () => ({
      app_type: 'claude-code',
      search: search.trim() || undefined,
      limit,
      offset,
    }),
    [search, limit, offset],
  );

  const { data, isLoading, error } = useQuery({
    queryKey: ['sessions', params],
    queryFn: () => sessionsApi.list(params),
  });

  const { data: detail, isLoading: detailLoading, error: detailError } = useQuery({
    queryKey: ['sessions', 'messages', selected?.source_path],
    queryFn: () => sessionsApi.messages(selected!.source_path),
    enabled: !!selected,
  });

  const items = data?.items ?? [];
  const total = data?.total ?? 0;
  const totalPages = Math.max(1, Math.ceil(total / limit));
  const currentPage = Math.floor(offset / limit) + 1;

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">会话</h1>
        <p className="text-sm text-gray-500 mt-1">
          只读浏览 Claude Code 本地会话 JSONL，不删除、不修改、不执行恢复命令。
        </p>
      </div>

      <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-md p-3 text-sm text-amber-700 dark:text-amber-300">
        会话内容可能包含 API Key、路径、命令输出或项目隐私。复制、截图或分享前请先确认内容安全。
      </div>

      <div className="flex flex-wrap gap-3 items-end">
        <div className="flex-1 min-w-64">
          <label className="block text-xs text-gray-500 mb-1">搜索</label>
          <input
            value={search}
            onChange={(e) => {
              setSearch(e.target.value);
              setOffset(0);
            }}
            placeholder="搜索标题、摘要、项目目录、会话 ID 或文件路径"
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-white dark:bg-gray-900"
          />
        </div>
        <div>
          <label className="block text-xs text-gray-500 mb-1">每页</label>
          <select
            value={limit}
            onChange={(e) => {
              setLimit(Number(e.target.value));
              setOffset(0);
            }}
            className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-white dark:bg-gray-900"
          >
            <option value={20}>20</option>
            <option value={50}>50</option>
            <option value={100}>100</option>
          </select>
        </div>
      </div>

      {data && (
        <div className="text-xs text-gray-500 flex flex-wrap gap-4">
          <span>
            扫描目录：<span className="font-mono">{data.scan_root}</span>
          </span>
          <span>共 {total} 个会话</span>
        </div>
      )}

      {data?.warnings.map((warning) => (
        <div
          key={warning}
          className="bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 rounded-md p-3 text-sm text-yellow-700 dark:text-yellow-300"
        >
          {warning}
        </div>
      ))}

      <div className="grid grid-cols-1 xl:grid-cols-[minmax(0,480px)_minmax(0,1fr)] gap-6">
        <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 overflow-hidden">
          {isLoading && <p className="p-4 text-gray-500">加载中...</p>}
          {error && <p className="p-4 text-red-500">加载失败: {(error as Error).message}</p>}
          {!isLoading && !error && items.length === 0 && (
            <div className="p-8 text-center text-gray-400 text-sm space-y-1">
              <p>暂无会话记录</p>
              <p>如果 {data?.scan_root ? <span className="font-mono">{data.scan_root}</span> : '~/.claude/projects'} 不存在，说明本机还没有 Claude Code 会话。</p>
            </div>
          )}

          {items.length > 0 && (
            <div className="divide-y divide-gray-100 dark:divide-gray-800 max-h-[720px] overflow-y-auto">
              {items.map((item) => (
                <SessionListItem
                  key={item.source_path}
                  item={item}
                  active={selected?.source_path === item.source_path}
                  onClick={() => setSelected(item)}
                />
              ))}
            </div>
          )}

          {total > limit && (
            <div className="flex items-center justify-between px-4 py-3 border-t border-gray-100 dark:border-gray-800 text-sm">
              <span className="text-gray-500">
                第 {currentPage}/{totalPages} 页
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

        <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 overflow-hidden min-h-[520px]">
          {!selected && (
            <p className="p-8 text-center text-gray-400 text-sm">点击左侧会话查看消息详情</p>
          )}
          {selected && (
            <div className="flex flex-col max-h-[760px]">
              <div className="border-b border-gray-100 dark:border-gray-800 p-4 space-y-3">
                <div>
                  <h2 className="font-semibold break-words">{selected.title}</h2>
                  <p className="text-xs text-gray-500 font-mono break-all mt-1">{selected.session_id}</p>
                </div>
                <div className="grid grid-cols-1 md:grid-cols-2 gap-2 text-xs text-gray-500">
                  <Field label="项目" value={selected.project_dir ?? '—'} mono />
                  <Field label="活跃时间" value={formatSessionTime(selected.last_active_at_ms ?? selected.created_at_ms)} />
                  <Field label="文件" value={selected.source_path} mono />
                  <Field label="恢复命令" value={selected.resume_command ?? '—'} mono copyValue={selected.resume_command ?? undefined} />
                </div>
                {selected.summary && <p className="text-sm text-gray-600 dark:text-gray-300">{selected.summary}</p>}
                {selected.warnings.map((warning) => (
                  <p key={warning} className="text-xs text-yellow-600 dark:text-yellow-300">{warning}</p>
                ))}
              </div>

              <div className="flex-1 overflow-y-auto p-4 space-y-3">
                {detailLoading && <p className="text-gray-500 text-sm">消息加载中...</p>}
                {detailError && <p className="text-red-500 text-sm">消息加载失败: {(detailError as Error).message}</p>}
                {detail?.warnings.map((warning) => (
                  <div
                    key={warning}
                    className="bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 rounded-md p-2 text-xs text-yellow-700 dark:text-yellow-300"
                  >
                    {warning}
                  </div>
                ))}
                {detail && detail.messages.length === 0 && (
                  <p className="text-gray-400 text-sm">没有可展示的消息。</p>
                )}
                {detail?.messages.map((message, index) => (
                  <MessageCard key={`${message.timestamp_ms ?? 'no-ts'}-${index}`} message={message} />
                ))}
              </div>
            </div>
          )}
        </div>
      </div>
    </div>
  );
}

function SessionListItem({
  item,
  active,
  onClick,
}: {
  item: SessionMeta;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <button
      onClick={onClick}
      className={`w-full text-left px-4 py-3 hover:bg-gray-50 dark:hover:bg-gray-800/50 transition-colors ${
        active ? 'bg-blue-50 dark:bg-blue-900/10' : ''
      }`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0">
          <p className="text-sm font-medium truncate">{item.title}</p>
          <p className="text-xs text-gray-500 truncate mt-1">{item.project_dir ?? item.source_path}</p>
        </div>
        <span className="text-xs text-gray-400 shrink-0">{formatSessionTime(item.last_active_at_ms ?? item.created_at_ms)}</span>
      </div>
      <div className="flex items-center gap-2 mt-2 text-xs text-gray-500">
        <span className="font-mono truncate">{item.session_id}</span>
        {item.warnings.length > 0 && <span className="text-yellow-600 dark:text-yellow-300">有警告</span>}
      </div>
      {item.summary && <p className="text-xs text-gray-500 mt-2 line-clamp-2">{item.summary}</p>}
    </button>
  );
}

function MessageCard({ message }: { message: SessionMessage }) {
  const [expanded, setExpanded] = useState(false);
  const isLong = message.content.length > LONG_CONTENT_LENGTH;
  const visible = isLong && !expanded ? `${message.content.slice(0, LONG_CONTENT_LENGTH)}...` : message.content;

  return (
    <div className="rounded-lg border border-gray-200 dark:border-gray-800 p-3 space-y-2">
      <div className="flex items-center justify-between gap-3 text-xs">
        <div className="flex items-center gap-2 min-w-0">
          <RoleBadge role={message.role} />
          {message.raw_kind && <span className="text-gray-400 font-mono truncate">{message.raw_kind}</span>}
          <span className="text-gray-400 shrink-0">{formatSessionTime(message.timestamp_ms)}</span>
        </div>
        <button
          onClick={() => copyText(message.content)}
          className="px-2 py-1 rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800"
        >
          复制
        </button>
      </div>
      <pre className="text-sm whitespace-pre-wrap break-words font-sans text-gray-800 dark:text-gray-100 bg-gray-50 dark:bg-gray-950 rounded p-3 overflow-x-auto">
        {visible}
      </pre>
      {isLong && (
        <button
          onClick={() => setExpanded((v) => !v)}
          className="text-xs text-blue-600 dark:text-blue-400 hover:underline"
        >
          {expanded ? '收起长消息' : '展开长消息'}
        </button>
      )}
    </div>
  );
}

function RoleBadge({ role }: { role: string }) {
  const label = roleLabel(role);
  const cls =
    role === 'user'
      ? 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300'
      : role === 'assistant'
        ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-300'
        : role === 'tool'
          ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-300'
          : 'bg-gray-100 text-gray-700 dark:bg-gray-800 dark:text-gray-300';
  return <span className={`px-1.5 py-0.5 rounded ${cls}`}>{label}</span>;
}

function Field({
  label,
  value,
  mono,
  copyValue,
}: {
  label: string;
  value: string;
  mono?: boolean;
  copyValue?: string;
}) {
  return (
    <div className="flex items-start gap-2 min-w-0">
      <span className="text-gray-400 w-16 shrink-0">{label}</span>
      <span className={`${mono ? 'font-mono' : ''} break-all min-w-0 flex-1`}>{value}</span>
      {copyValue && (
        <button
          onClick={() => copyText(copyValue)}
          className="px-1.5 py-0.5 rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 shrink-0"
        >
          复制
        </button>
      )}
    </div>
  );
}

function formatSessionTime(ms: number | null | undefined): string {
  if (ms == null) return '—';
  const d = new Date(ms);
  if (Number.isNaN(d.getTime())) return '—';
  return d.toLocaleString();
}

function roleLabel(role: string): string {
  switch (role) {
    case 'user':
      return '用户';
    case 'assistant':
      return '助手';
    case 'tool':
      return '工具';
    case 'system':
      return '系统';
    default:
      return role;
  }
}

function copyText(text: string) {
  navigator.clipboard?.writeText(text).catch(() => {});
}
