import { useState } from 'react';
import type {
  CreateProviderBody,
  Provider,
  ProviderMode,
  UpdateProviderBody,
} from '../../lib/api';

interface ProviderFormProps {
  /** 传入则进入编辑模式，否则为创建模式。 */
  initial?: Provider | null;
  onSubmit: (body: CreateProviderBody | UpdateProviderBody, isEdit: boolean) => void;
  onCancel: () => void;
  pending?: boolean;
  /** 表单级错误（提交失败时由父层注入）。 */
  error?: string | null;
}

const MODES: ProviderMode[] = ['proxy', 'direct'];

/**
 * Provider 创建/编辑表单（模态）。
 *
 * `settings_config` 用 JSON textarea 直接编辑文本；提交前校验是否合法 JSON。
 * direct 模式的 endpoint_id 选择器留 P1 后深度绑定，本期直接在 JSON 中手填。
 */
export function ProviderForm({
  initial,
  onSubmit,
  onCancel,
  pending = false,
  error = null,
}: ProviderFormProps) {
  const isEdit = !!initial;
  const [name, setName] = useState(initial?.name ?? '');
  const [mode, setMode] = useState<ProviderMode>(initial?.mode ?? 'proxy');
  const [category, setCategory] = useState(initial?.category ?? '');
  const [notes, setNotes] = useState(initial?.notes ?? '');
  const [settingsText, setSettingsText] = useState(
    () => JSON.stringify(initial?.settings_config ?? {}, null, 2),
  );
  const [appType, setAppType] = useState<'claude-code' | 'codex'>(
    (initial?.app_type as 'claude-code' | 'codex') ?? 'claude-code',
  );
  const [jsonError, setJsonError] = useState<string | null>(null);

  function handleSubmit() {
    if (!name.trim()) {
      setJsonError(null);
      return;
    }
    let settingsConfig: unknown;
    try {
      settingsConfig = settingsText.trim() === '' ? {} : JSON.parse(settingsText);
    } catch (e) {
      setJsonError(`settings_config 不是合法 JSON: ${e instanceof Error ? e.message : String(e)}`);
      return;
    }
    setJsonError(null);

    if (isEdit) {
      const body: UpdateProviderBody = {
        name: name.trim(),
        mode,
        settings_config: settingsConfig,
        category: category.trim() || null,
        notes: notes.trim() || null,
      };
      onSubmit(body, true);
    } else {
      const body: CreateProviderBody = {
        app_type: appType,
        name: name.trim(),
        mode,
        settings_config: settingsConfig,
        category: category.trim() || null,
        notes: notes.trim() || null,
      };
      onSubmit(body, false);
    }
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onClick={onCancel}
    >
      <div
        className="w-full max-w-lg rounded-lg bg-white dark:bg-gray-900 border border-gray-200 dark:border-gray-800 p-5 space-y-3 shadow-lg"
        onClick={(e) => e.stopPropagation()}
      >
        <h2 className="font-semibold text-lg">
          {isEdit ? '编辑 provider' : '添加 provider'}
        </h2>

        <div className="grid grid-cols-2 gap-3">
          <div>
            <label className="block text-xs text-gray-500 mb-1">名称</label>
            <input
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
              placeholder="例如：备用 Anthropic"
            />
          </div>
          <div>
            <label className="block text-xs text-gray-500 mb-1">app_type</label>
            <select
              value={appType}
              onChange={(e) => setAppType(e.target.value as 'claude-code' | 'codex')}
              disabled={isEdit}
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent disabled:opacity-60"
            >
              <option value="claude-code">claude-code</option>
              <option value="codex">codex</option>
            </select>
          </div>
          <div>
            <label className="block text-xs text-gray-500 mb-1">模式</label>
            <select
              value={mode}
              onChange={(e) => setMode(e.target.value as ProviderMode)}
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
            >
              {MODES.map((m) => (
                <option key={m} value={m}>
                  {m === 'proxy' ? '代理 (proxy)' : '直连 (direct)'}
                </option>
              ))}
            </select>
          </div>
          <div>
            <label className="block text-xs text-gray-500 mb-1">分类（可选）</label>
            <input
              value={category}
              onChange={(e) => setCategory(e.target.value)}
              className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
              placeholder="例如：official"
            />
          </div>
        </div>

        <div>
          <label className="block text-xs text-gray-500 mb-1">备注（可选）</label>
          <textarea
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            rows={2}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent"
            placeholder="用途、凭据来源等说明"
          />
        </div>

        <div>
          <label className="block text-xs text-gray-500 mb-1">
            settings_config（JSON）
            {mode === 'direct' && (
              <span className="ml-1 text-gray-400">
                direct 模式可在 JSON 中填 endpoint_id
              </span>
            )}
          </label>
          <textarea
            value={settingsText}
            onChange={(e) => setSettingsText(e.target.value)}
            rows={6}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-xs bg-transparent font-mono"
            placeholder='{"endpoint_id":"..."}'
          />
          {jsonError && <p className="text-xs text-red-500 mt-1">{jsonError}</p>}
        </div>

        {error && (
          <p className="text-sm text-red-500">操作失败: {error}</p>
        )}

        <div className="flex gap-2 justify-end pt-2">
          <button
            type="button"
            onClick={onCancel}
            className="px-4 py-2 bg-gray-100 dark:bg-gray-800 rounded-md text-sm hover:bg-gray-200 dark:hover:bg-gray-700"
          >
            取消
          </button>
          <button
            type="button"
            onClick={handleSubmit}
            disabled={!name.trim() || pending}
            className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
          >
            {pending ? '保存中...' : isEdit ? '保存' : '创建'}
          </button>
        </div>
      </div>
    </div>
  );
}
