import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import {
  mcpApi,
  type CreateMcpBody,
  type McpServer,
  type UpdateMcpBody,
} from '../lib/api';

/** MCP 规范编辑表单的本地状态。 */
interface FormState {
  open: boolean;
  /** 编辑时的原 server（null = 新建）。 */
  initial: McpServer | null;
}

const EMPTY_SPEC = `{
  "command": "npx",
  "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path"]
}`;

export function McpPage() {
  const queryClient = useQueryClient();
  const { data: servers, isLoading, error } = useQuery({
    queryKey: ['mcp'],
    queryFn: mcpApi.list,
  });
  const { data: status } = useQuery({
    queryKey: ['mcp', 'status'],
    queryFn: mcpApi.status,
  });

  const [form, setForm] = useState<FormState>({ open: false, initial: null });
  const [banner, setBanner] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null);

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ['mcp'] });
  };

  const toggle = useMutation({
    mutationFn: (s: McpServer) => mcpApi.update(s.id, { enabled_claude: !s.enabled_claude }),
    onSuccess: invalidate,
    onError: (e: Error) => setBanner({ kind: 'err', text: `切换失败：${e.message}` }),
  });

  const remove = useMutation({
    mutationFn: (id: string) => mcpApi.remove(id),
    onSuccess: () => {
      invalidate();
      setBanner({ kind: 'ok', text: '已删除并同步。' });
    },
    onError: (e: Error) => setBanner({ kind: 'err', text: `删除失败：${e.message}` }),
  });

  const doImport = useMutation({
    mutationFn: () => mcpApi.import(),
    onSuccess: (report) => {
      invalidate();
      const skipped = report.skipped.length ? `，跳过 ${report.skipped.length} 项无效项` : '';
      setBanner({ kind: 'ok', text: `已从 ~/.claude.json 导入 ${report.imported} 项${skipped}。` });
    },
    onError: (e: Error) => setBanner({ kind: 'err', text: `导入失败：${e.message}` }),
  });

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between">
        <div>
          <h1 className="text-2xl font-bold">MCP 服务器</h1>
          <p className="text-sm text-gray-500 mt-1">
            管理 Claude Code 的 MCP 服务器清单。启用的服务器会即时同步写入{' '}
            <span className="font-mono text-xs">~/.claude.json</span>。
          </p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => doImport.mutate()}
            disabled={doImport.isPending}
            className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50"
          >
            {doImport.isPending ? '导入中...' : '从 ~/.claude.json 导入'}
          </button>
          <button
            onClick={() => setForm({ open: true, initial: null })}
            className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700"
          >
            新建服务器
          </button>
        </div>
      </div>

      {/* 全量投影提示 */}
      <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-md p-3 text-xs text-amber-700 dark:text-amber-300 space-y-1">
        <p className="font-medium">同步说明</p>
        <ul className="list-disc list-inside space-y-0.5">
          <li>启用项整体投影到 <span className="font-mono">mcpServers</span> 字段，其它顶层键保留。</li>
          <li>直接手改进 <span className="font-mono">~/.claude.json</span> 但不在此清单的服务器，下次同步会被覆盖——请先用「导入」把它们纳管。</li>
        </ul>
      </div>

      {status && (
        <div className="text-xs text-gray-500 flex gap-4">
          <span>
            配置文件：<span className="font-mono">{status.config_path}</span>
          </span>
          <span>{status.config_exists ? `live 已有 ${status.live_server_count} 项` : 'live 尚未创建'}</span>
        </div>
      )}

      {banner && (
        <div
          className={`rounded-md p-3 text-sm ${
            banner.kind === 'ok'
              ? 'bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 text-green-700 dark:text-green-300'
              : 'bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-300 whitespace-pre-wrap'
          }`}
        >
          {banner.text}
        </div>
      )}

      {isLoading && <p className="text-gray-500">加载中...</p>}
      {error && <p className="text-red-500">加载失败: {(error as Error).message}</p>}

      {servers && servers.length === 0 && (
        <p className="text-gray-500 text-sm">还没有 MCP 服务器。点「新建服务器」或从 ~/.claude.json 导入。</p>
      )}

      {servers && servers.length > 0 && (
        <div className="space-y-2">
          {servers.map((s) => (
            <div
              key={s.id}
              className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 flex items-start justify-between gap-4"
            >
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-medium">{s.name}</span>
                  {s.enabled_claude && (
                    <span className="text-xs px-1.5 py-0.5 rounded bg-green-100 dark:bg-green-900/40 text-green-700 dark:text-green-300">
                      已启用
                    </span>
                  )}
                </div>
                {s.description && (
                  <p className="text-xs text-gray-500 mt-0.5">{s.description}</p>
                )}
                <pre className="text-xs font-mono text-gray-600 dark:text-gray-400 mt-2 bg-gray-50 dark:bg-gray-800 rounded p-2 overflow-x-auto">
                  {JSON.stringify(s.server_config, null, 2)}
                </pre>
              </div>
              <div className="flex flex-col gap-2 items-end shrink-0">
                <button
                  onClick={() => toggle.mutate(s)}
                  disabled={toggle.isPending}
                  className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                    s.enabled_claude ? 'bg-green-600' : 'bg-gray-300 dark:bg-gray-700'
                  }`}
                  title={s.enabled_claude ? '点击禁用' : '点击启用'}
                >
                  <span
                    className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                      s.enabled_claude ? 'translate-x-6' : 'translate-x-1'
                    }`}
                  />
                </button>
                <div className="flex gap-1">
                  <button
                    onClick={() => setForm({ open: true, initial: s })}
                    className="px-2 py-1 text-xs border border-gray-300 dark:border-gray-700 rounded hover:bg-gray-50 dark:hover:bg-gray-800"
                  >
                    编辑
                  </button>
                  <button
                    onClick={() => {
                      if (confirm(`删除 MCP 服务器「${s.name}」？`)) remove.mutate(s.id);
                    }}
                    className="px-2 py-1 text-xs border border-red-300 dark:border-red-800 text-red-600 dark:text-red-400 rounded hover:bg-red-50 dark:hover:bg-red-900/20"
                  >
                    删除
                  </button>
                </div>
              </div>
            </div>
          ))}
        </div>
      )}

      {form.open && (
        <McpForm
          initial={form.initial}
          onClose={() => setForm({ open: false, initial: null })}
          onSaved={(msg) => {
            invalidate();
            setForm({ open: false, initial: null });
            setBanner({ kind: 'ok', text: msg });
          }}
        />
      )}
    </div>
  );
}

// ── 新建/编辑表单弹窗 ─────────────────────────────────────

function McpForm({
  initial,
  onClose,
  onSaved,
}: {
  initial: McpServer | null;
  onClose: () => void;
  onSaved: (msg: string) => void;
}) {
  const isEdit = initial !== null;
  const [name, setName] = useState(initial?.name ?? '');
  const [description, setDescription] = useState(initial?.description ?? '');
  const [enabled, setEnabled] = useState(initial?.enabled_claude ?? true);
  const [specText, setSpecText] = useState(
    initial ? JSON.stringify(initial.server_config, null, 2) : EMPTY_SPEC,
  );
  const [localError, setLocalError] = useState<string | null>(null);

  const save = useMutation({
    mutationFn: async () => {
      const spec = JSON.parse(specText);
      if (isEdit) {
        const body: UpdateMcpBody = {
          name,
          server_config: spec,
          description: description || null,
          enabled_claude: enabled,
        };
        await mcpApi.update(initial!.id, body);
        return;
      }
      const body: CreateMcpBody = {
        name,
        server_config: spec,
        description: description || null,
        enabled_claude: enabled,
      };
      await mcpApi.create(body);
    },
    onSuccess: () => onSaved(isEdit ? '已保存并同步。' : '已创建并同步。'),
    onError: (e: Error) => setLocalError(e.message),
  });

  const handleSave = () => {
    setLocalError(null);
    if (!name.trim()) {
      setLocalError('名称不能为空。');
      return;
    }
    try {
      JSON.parse(specText);
    } catch (e) {
      setLocalError(`规范不是合法 JSON：${e instanceof Error ? e.message : String(e)}`);
      return;
    }
    save.mutate();
  };

  return (
    <div className="fixed inset-0 bg-black/40 flex items-center justify-center z-50 p-4">
      <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 w-full max-w-lg max-h-[90vh] overflow-y-auto p-5 space-y-4">
        <h2 className="text-lg font-semibold">{isEdit ? '编辑 MCP 服务器' : '新建 MCP 服务器'}</h2>

        <div className="space-y-1">
          <label className="text-sm font-medium">名称</label>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="如 filesystem"
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-white dark:bg-gray-800"
          />
        </div>

        <div className="space-y-1">
          <label className="text-sm font-medium">描述（可选）</label>
          <input
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-white dark:bg-gray-800"
          />
        </div>

        <div className="space-y-1">
          <label className="text-sm font-medium">
            MCP 规范（裸 JSON：stdio 需 command，http/sse 需 url）
          </label>
          <textarea
            value={specText}
            onChange={(e) => setSpecText(e.target.value)}
            rows={10}
            spellCheck={false}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-xs font-mono bg-white dark:bg-gray-800"
          />
        </div>

        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={enabled}
            onChange={(e) => setEnabled(e.target.checked)}
          />
          启用（写入 ~/.claude.json）
        </label>

        {localError && (
          <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-md p-3 text-sm text-red-700 dark:text-red-300 whitespace-pre-wrap">
            {localError}
          </div>
        )}

        <div className="flex justify-end gap-2 pt-2">
          <button
            onClick={onClose}
            className="px-4 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm hover:bg-gray-50 dark:hover:bg-gray-800"
          >
            取消
          </button>
          <button
            onClick={handleSave}
            disabled={save.isPending}
            className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
          >
            {save.isPending ? '保存中...' : '保存'}
          </button>
        </div>
      </div>
    </div>
  );
}
