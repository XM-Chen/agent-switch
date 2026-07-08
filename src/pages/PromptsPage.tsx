import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import {
  promptsApi,
  type CreatePromptBody,
  type Prompt,
  type UpdatePromptBody,
} from '../lib/api';

/** Prompt 编辑表单的本地状态。 */
interface FormState {
  open: boolean;
  initial: Prompt | null;
}

/** content 摘要：取前 80 字符，换行折叠。 */
function summarize(content: string): string {
  const oneLine = content.replace(/\s+/g, ' ').trim();
  if (oneLine.length <= 80) return oneLine;
  return oneLine.slice(0, 80) + '…';
}

export function PromptsPage() {
  const queryClient = useQueryClient();
  const { data: prompts, isLoading, error } = useQuery({
    queryKey: ['prompts'],
    queryFn: promptsApi.list,
  });
  const { data: status } = useQuery({
    queryKey: ['prompts', 'status'],
    queryFn: promptsApi.status,
  });

  const [form, setForm] = useState<FormState>({ open: false, initial: null });
  const [banner, setBanner] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null);

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ['prompts'] });
  };

  // 单选启用语义：点击未启用 prompt → enable 它（自动取消其余）；点击已启用 → disable。
  const toggle = useMutation({
    mutationFn: (p: Prompt) => (p.enabled_claude ? promptsApi.disable(p.id) : promptsApi.enable(p.id)),
    onSuccess: invalidate,
    onError: (e: Error) => setBanner({ kind: 'err', text: `切换失败：${e.message}` }),
  });

  const remove = useMutation({
    mutationFn: (id: string) => promptsApi.remove(id),
    onSuccess: () => {
      invalidate();
      setBanner({ kind: 'ok', text: '已删除。' });
    },
    onError: (e: Error) => setBanner({ kind: 'err', text: `删除失败：${e.message}` }),
  });

  const doImport = useMutation({
    mutationFn: () => promptsApi.import(),
    onSuccess: (report) => {
      invalidate();
      setBanner({
        kind: 'ok',
        text:
          report.imported > 0
            ? `已从 ~/.claude/CLAUDE.md 导入 ${report.imported} 项。`
            : 'live CLAUDE.md 为空或不存在，未导入。',
      });
    },
    onError: (e: Error) => setBanner({ kind: 'err', text: `导入失败：${e.message}` }),
  });

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between">
        <div>
          <h1 className="text-2xl font-bold">Prompts</h1>
          <p className="text-sm text-gray-500 mt-1">
            管理 Claude Code 的提示词清单。任一时刻至多一份「激活」并投影写入{' '}
            <span className="font-mono text-xs">~/.claude/CLAUDE.md</span>。启用前会自动回填
            live 手改内容，避免覆盖丢失。
          </p>
        </div>
        <div className="flex gap-2">
          <button
            onClick={() => doImport.mutate()}
            disabled={doImport.isPending}
            className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50"
          >
            {doImport.isPending ? '导入中...' : '从 CLAUDE.md 导入'}
          </button>
          <button
            onClick={() => setForm({ open: true, initial: null })}
            className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700"
          >
            新建提示词
          </button>
        </div>
      </div>

      {/* 单激活说明 */}
      <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-md p-3 text-xs text-amber-700 dark:text-amber-300 space-y-1">
        <p className="font-medium">单激活 + 回填保护</p>
        <ul className="list-disc list-inside space-y-0.5">
          <li>启用新 prompt 前，若当前已有激活项，会把 live 手改内容回填进该项再切换。</li>
          <li>若 DB 无激活项且 live 有未纳入的原文，会自动建一份「原始提示词」备份。</li>
          <li>直接手改 <span className="font-mono">~/.claude/CLAUDE.md</span> 不会被立即覆盖，下次启用时才捕获回填。</li>
        </ul>
      </div>

      {status && (
        <div className="text-xs text-gray-500 flex gap-4">
          <span>
            配置文件：<span className="font-mono">{status.config_path}</span>
          </span>
          <span>{status.config_exists ? 'live 已存在' : 'live 尚未创建'}</span>
          <span>
            当前激活：{status.active_prompt_id ? '有' : '无'}
          </span>
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

      {prompts && prompts.length === 0 && (
        <p className="text-gray-500 text-sm">还没有提示词。点「新建提示词」或从 CLAUDE.md 导入。</p>
      )}

      {prompts && prompts.length > 0 && (
        <div className="space-y-2">
          {prompts.map((p) => (
            <div
              key={p.id}
              className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 flex items-start justify-between gap-4"
            >
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2">
                  <span className="font-medium">{p.name}</span>
                  {p.enabled_claude && (
                    <span className="text-xs px-1.5 py-0.5 rounded bg-green-100 dark:bg-green-900/40 text-green-700 dark:text-green-300">
                      已激活
                    </span>
                  )}
                </div>
                <p className="text-xs text-gray-500 mt-0.5 break-words">
                  {summarize(p.content) || <span className="italic text-gray-400">（空内容）</span>}
                </p>
                {p.description && (
                  <p className="text-xs text-gray-400 mt-1">{p.description}</p>
                )}
              </div>
              <div className="flex flex-col gap-2 items-end shrink-0">
                <button
                  onClick={() => toggle.mutate(p)}
                  disabled={toggle.isPending}
                  className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                    p.enabled_claude ? 'bg-green-600' : 'bg-gray-300 dark:bg-gray-700'
                  }`}
                  title={p.enabled_claude ? '点击禁用' : '点击激活（取消其余）'}
                >
                  <span
                    className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                      p.enabled_claude ? 'translate-x-6' : 'translate-x-1'
                    }`}
                  />
                </button>
                <div className="flex gap-1">
                  <button
                    onClick={() => setForm({ open: true, initial: p })}
                    className="px-2 py-1 text-xs border border-gray-300 dark:border-gray-700 rounded hover:bg-gray-50 dark:hover:bg-gray-800"
                  >
                    编辑
                  </button>
                  <button
                    onClick={() => {
                      if (p.enabled_claude) {
                        setBanner({
                          kind: 'err',
                          text: '不能删除已激活的提示词，请先禁用。',
                        });
                        return;
                      }
                      if (confirm(`删除提示词「${p.name}」？`)) remove.mutate(p.id);
                    }}
                    disabled={p.enabled_claude}
                    className="px-2 py-1 text-xs border border-red-300 dark:border-red-800 text-red-600 dark:text-red-400 rounded hover:bg-red-50 dark:hover:bg-red-900/20 disabled:opacity-40 disabled:cursor-not-allowed"
                    title={p.enabled_claude ? '激活项不可删除，请先禁用' : '删除'}
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
        <PromptForm
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

function PromptForm({
  initial,
  onClose,
  onSaved,
}: {
  initial: Prompt | null;
  onClose: () => void;
  onSaved: (msg: string) => void;
}) {
  const isEdit = initial !== null;
  const [name, setName] = useState(initial?.name ?? '');
  const [description, setDescription] = useState(initial?.description ?? '');
  const [content, setContent] = useState(initial?.content ?? '');
  const [localError, setLocalError] = useState<string | null>(null);

  const save = useMutation({
    mutationFn: async () => {
      if (isEdit) {
        const body: UpdatePromptBody = {
          name,
          content,
          description: description || null,
        };
        await promptsApi.update(initial!.id, body);
        return;
      }
      const body: CreatePromptBody = {
        name,
        content,
        description: description || null,
      };
      await promptsApi.create(body);
    },
    onSuccess: () => onSaved(isEdit ? '已保存。' : '已创建。'),
    onError: (e: Error) => setLocalError(e.message),
  });

  const handleSave = () => {
    setLocalError(null);
    if (!name.trim()) {
      setLocalError('名称不能为空。');
      return;
    }
    save.mutate();
  };

  return (
    <div className="fixed inset-0 bg-black/40 flex items-center justify-center z-50 p-4">
      <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 w-full max-w-lg max-h-[90vh] overflow-y-auto p-5 space-y-4">
        <h2 className="text-lg font-semibold">{isEdit ? '编辑提示词' : '新建提示词'}</h2>

        <div className="space-y-1">
          <label className="text-sm font-medium">名称</label>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="如 code-review"
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
          <label className="text-sm font-medium">提示词内容（明文 Markdown，写 live 原样投影）</label>
          <textarea
            value={content}
            onChange={(e) => setContent(e.target.value)}
            rows={12}
            spellCheck={false}
            placeholder="# 提示词&#10;&#10;在这里输入 Claude Code 的 CLAUDE.md 内容..."
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-xs font-mono bg-white dark:bg-gray-800"
          />
        </div>

        <p className="text-xs text-gray-500">
          {isEdit
            ? '保存不改变激活态。如需切换激活，请在列表里点开关。'
            : '新建默认不激活。如需激活，请在列表里点开关。'}
        </p>

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
