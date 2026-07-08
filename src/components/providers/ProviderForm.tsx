import { useRef, useState } from 'react';
import type {
  CreateProviderBody,
  Provider,
  ProviderMode,
  UpdateProviderBody,
} from '../../lib/api';
import {
  CLAUDE_PROVIDER_PRESETS,
  type ClaudeProviderPreset,
} from '../../config/claudeProviderPresets';
import {
  hasClaudeOneMMarker,
  parseClaudeEnv,
  serializeClaudeEnv,
  setClaudeOneMMarker,
  stripClaudeOneMMarker,
  validateApiTimeoutMs,
  type ClaudeEnvSwitches,
} from './claudeEnvHelpers';

interface ProviderFormProps {
  /** 传入则进入编辑模式，否则为创建模式。 */
  initial?: Provider | null;
  onSubmit: (body: CreateProviderBody | UpdateProviderBody, isEdit: boolean) => void;
  onCancel: () => void;
  pending?: boolean;
  /** 表单级错误（提交失败时由父层注入）。 */
  error?: string | null;
  /**
   * 「应用到 live」：仅当前激活 provider（is_current）显示该按钮时点击触发。
   * 父层先保存（onSubmit 流）再调用 `providersApi.switch(id)` 重切，把更新后的
   * meta.snapshot.env 落 live。未传则不显示该按钮。
   */
  onApplyLive?: () => void;
  /** 「应用到 live」按钮是否在 pending（避免并发点击）。 */
  applyLivePending?: boolean;
}

const MODES: ProviderMode[] = ['proxy', 'direct'];

const EMPTY_ENV_SWITCHES: ClaudeEnvSwitches = {
  haikuModel: '',
  haikuModelName: '',
  sonnetModel: '',
  sonnetModelName: '',
  opusModel: '',
  opusModelName: '',
  fallbackModel: '',
  apiTimeout: '',
  useBedrock: false,
  awsRegion: '',
  awsAccessKeyId: '',
  awsSecretAccessKey: '',
  disableNonessentialTraffic: false,
  maxOutputTokens: '',
  disableExperimentalBetas: false,
};

/**
 * Provider 创建/编辑表单（模态）。
 *
 * `settings_config` 用 JSON textarea 直接编辑文本；提交前校验是否合法 JSON。
 * direct 模式的 endpoint_id 选择器留 P1 后深度绑定，本期直接在 JSON 中手填。
 *
 * Claude Code provider 额外有「行为开关」分区：结构化编辑 `meta.snapshot.env` 中的
 * 非连接 env 键（模型三档/超时/Bedrock 等）+ 裸 JSON 逃生舱 + 预设预填。
 */
export function ProviderForm({
  initial,
  onSubmit,
  onCancel,
  pending = false,
  error = null,
  onApplyLive,
  applyLivePending = false,
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

  // ── Claude Code 行为开关：结构化字段 + 裸 JSON 逃生舱 ──
  const [envSwitches, setEnvSwitches] = useState<ClaudeEnvSwitches>(() =>
    initial?.app_type === 'claude-code' ? parseClaudeEnv(initial?.meta) : EMPTY_ENV_SWITCHES,
  );
  // 裸 JSON 逃生舱：meta.snapshot.env 的全文（保留未结构化键）
  const [envRawText, setEnvRawText] = useState(() => {
    if (initial?.app_type !== 'claude-code') return '{}';
    const meta = (initial?.meta ?? {}) as Record<string, unknown>;
    const snapshot = (meta?.snapshot ?? {}) as Record<string, unknown>;
    const env = (snapshot?.env ?? {}) as unknown;
    return JSON.stringify(env, null, 2);
  });
  const [envRawError, setEnvRawError] = useState<string | null>(null);
  const [apiTimeoutError, setApiTimeoutError] = useState<string | null>(null);
  // 用户正在编辑裸 JSON 时，跳过一次结构化字段→裸 JSON 的同步覆盖。
  const isUserEditingRawRef = useRef(false);

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
      // Claude Code：合并结构化字段 + 裸 JSON 逃生舱 → meta.snapshot.env
      if (appType === 'claude-code') {
        // API_TIMEOUT_MS：空串 = 删键（放行），非空须为正整数（Claude Code 行为依赖）
        const timeoutError = validateApiTimeoutMs(envSwitches.apiTimeout);
        if (timeoutError) {
          setApiTimeoutError(timeoutError);
          return;
        }
        setApiTimeoutError(null);
        body.meta = mergeEnvIntoMeta(initial?.meta, envSwitches, envRawText);
      }
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

  // ── 结构化字段 → 裸 JSON 逃生舱：同步 ──
  // 用户编辑结构化字段 → 视为离开裸 JSON 编辑态，恢复同步（否则裸 JSON 会停在
  // 上一次编辑的陈旧文本，bidi 同步失效）。
  function updateSwitches(patch: Partial<ClaudeEnvSwitches>) {
    isUserEditingRawRef.current = false;
    if ('apiTimeout' in patch) {
      setApiTimeoutError(null);
    }
    setEnvSwitches((prev) => {
      const next = { ...prev, ...patch };
      // 同步到裸 JSON：以最新结构化字段序列化 env，保留逃生舱内非结构化键
      const merged = mergeEnvIntoMeta(initial?.meta, next, envRawText);
      const meta = merged as Record<string, unknown>;
      const snapshot = (meta?.snapshot ?? {}) as Record<string, unknown>;
      const env = (snapshot?.env ?? {}) as unknown;
      setEnvRawText(JSON.stringify(env, null, 2));
      return next;
    });
  }

  // ── 裸 JSON 逃生舱 → 结构化字段：用户编辑裸 JSON 时解析回填 ──
  function updateEnvRawText(text: string) {
    isUserEditingRawRef.current = true;
    setEnvRawText(text);
    let envObj: Record<string, unknown> = {};
    try {
      const parsed = text.trim() === '' ? {} : JSON.parse(text);
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        envObj = parsed as Record<string, unknown>;
      } else {
        throw new Error('必须是 JSON 对象');
      }
    } catch (e) {
      setEnvRawError(`env 不是合法 JSON 对象: ${e instanceof Error ? e.message : String(e)}`);
      return;
    }
    setEnvRawError(null);
    // 以裸 JSON 为基底解析结构化字段
    const fakeMeta = { snapshot: { env: envObj } };
    setEnvSwitches(parseClaudeEnv(fakeMeta));
  }

  // ── 预设选择：把预设 env 合并进结构化字段 + 裸 JSON ──
  function handlePresetSelect(preset: ClaudeProviderPreset) {
    // 1. 以当前 envRawText 为基底（保留逃生舱内未在预设中的键），再叠加预设
    const rawBase = parseRawEnvSafe(envRawText);
    const mergedEnv: Record<string, unknown> = { ...rawBase };
    for (const [k, v] of Object.entries(preset.env)) {
      if (v.trim()) mergedEnv[k] = v;
      else delete mergedEnv[k];
    }
    const fakeMeta = { snapshot: { env: mergedEnv } };
    setEnvRawText(JSON.stringify(mergedEnv, null, 2));
    setEnvSwitches(parseClaudeEnv(fakeMeta));
    setEnvRawError(null);
    isUserEditingRawRef.current = false;
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onClick={onCancel}
    >
      <div
        className="w-full max-w-lg rounded-lg bg-white dark:bg-gray-900 border border-gray-200 dark:border-gray-800 p-5 space-y-3 shadow-lg max-h-[90vh] overflow-y-auto"
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

        {appType === 'claude-code' && (
          <ClaudeEnvSwitchesSection
            switches={envSwitches}
            envRawText={envRawText}
            envRawError={envRawError}
            apiTimeoutError={apiTimeoutError}
            onUpdateSwitches={updateSwitches}
            onUpdateEnvRaw={updateEnvRawText}
            onPresetSelect={handlePresetSelect}
          />
        )}

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

        {isEdit && initial?.is_current && onApplyLive && (
          <div className="border-t border-gray-200 dark:border-gray-800 pt-3 space-y-1">
            <p className="text-xs text-gray-500">
              该 provider 已激活。编辑保存后点「应用到 live」即时落盘，否则下次切换该 provider 时生效。
            </p>
            <button
              type="button"
              onClick={onApplyLive}
              disabled={applyLivePending}
              className="px-4 py-2 bg-green-600 text-white rounded-md text-sm hover:bg-green-700 disabled:opacity-50"
            >
              应用到 live
            </button>
          </div>
        )}
      </div>
    </div>
  );
}

// ── Claude Code 行为开关分区 ─────────────────────────────

interface ClaudeEnvSwitchesSectionProps {
  switches: ClaudeEnvSwitches;
  envRawText: string;
  envRawError: string | null;
  apiTimeoutError: string | null;
  onUpdateSwitches: (patch: Partial<ClaudeEnvSwitches>) => void;
  onUpdateEnvRaw: (text: string) => void;
  onPresetSelect: (preset: ClaudeProviderPreset) => void;
}

function ClaudeEnvSwitchesSection({
  switches,
  envRawText,
  envRawError,
  apiTimeoutError,
  onUpdateSwitches,
  onUpdateEnvRaw,
  onPresetSelect,
}: ClaudeEnvSwitchesSectionProps) {
  return (
    <div className="border border-gray-200 dark:border-gray-800 rounded-md p-3 space-y-3">
      <div className="flex items-center justify-between">
        <h3 className="text-sm font-medium">Claude Code 行为开关</h3>
        <select
          onChange={(e) => {
            const preset = CLAUDE_PROVIDER_PRESETS.find((p) => p.name === e.target.value);
            if (preset) onPresetSelect(preset);
            e.target.value = '';
          }}
          value=""
          className="px-2 py-1 border border-gray-300 dark:border-gray-700 rounded text-xs bg-transparent"
        >
          <option value="">选择预设预填…</option>
          {CLAUDE_PROVIDER_PRESETS.map((p) => (
            <option key={p.name} value={p.name}>
              {p.name}
            </option>
          ))}
        </select>
      </div>

      <p className="text-xs text-gray-500">
        这些 env 开关写入 provider 快照层，切换该 provider 时落 live
        <code className="mx-1">~/.claude/settings.json</code>。连接层
        <code className="mx-1">ANTHROPIC_BASE_URL</code>/
        <code className="mx-1">ANTHROPIC_AUTH_TOKEN</code> 由端点体系注入，不在此编辑。
      </p>

      {/* 模型三档（+ 显示名 + 1M 勾选） */}
      <ModelRoleRow
        label="Sonnet"
        model={switches.sonnetModel}
        modelName={switches.sonnetModelName}
        supportsOneM
        onModelChange={(v) => onUpdateSwitches({ sonnetModel: v })}
        onModelNameChange={(v) => onUpdateSwitches({ sonnetModelName: v })}
        onOneMChange={(enabled) =>
          onUpdateSwitches({
            sonnetModel: setClaudeOneMMarker(switches.sonnetModel, enabled),
          })
        }
      />
      <ModelRoleRow
        label="Opus"
        model={switches.opusModel}
        modelName={switches.opusModelName}
        supportsOneM
        onModelChange={(v) => onUpdateSwitches({ opusModel: v })}
        onModelNameChange={(v) => onUpdateSwitches({ opusModelName: v })}
        onOneMChange={(enabled) =>
          onUpdateSwitches({
            opusModel: setClaudeOneMMarker(switches.opusModel, enabled),
          })
        }
      />
      <ModelRoleRow
        label="Haiku"
        model={switches.haikuModel}
        modelName={switches.haikuModelName}
        supportsOneM={false}
        onModelChange={(v) => onUpdateSwitches({ haikuModel: v })}
        onModelNameChange={(v) => onUpdateSwitches({ haikuModelName: v })}
        onOneMChange={() => undefined}
      />

      <div>
        <label className="block text-xs text-gray-500 mb-1">兜底模型 ANTHROPIC_MODEL</label>
        <input
          value={stripClaudeOneMMarker(switches.fallbackModel)}
          onChange={(e) => onUpdateSwitches({ fallbackModel: e.target.value })}
          className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
          placeholder="例如：deepseek-chat"
        />
      </div>

      <div>
        <label className="block text-xs text-gray-500 mb-1">API_TIMEOUT_MS（毫秒）</label>
        <input
          value={switches.apiTimeout}
          onChange={(e) => onUpdateSwitches({ apiTimeout: e.target.value })}
          className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
          placeholder="例如：60000"
          inputMode="numeric"
        />
        {apiTimeoutError && <p className="text-xs text-red-500 mt-1">{apiTimeoutError}</p>}
      </div>

      {/* Bedrock 开关 + AWS 凭证（明文，用户拍板） */}
      <div className="border-t border-gray-200 dark:border-gray-800 pt-3 space-y-2">
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={switches.useBedrock}
            onChange={(e) => onUpdateSwitches({ useBedrock: e.target.checked })}
          />
          启用 AWS Bedrock（CLAUDE_CODE_USE_BEDROCK）
        </label>
        {switches.useBedrock && (
          <>
            <div>
              <label className="block text-xs text-gray-500 mb-1">AWS_REGION</label>
              <input
                value={switches.awsRegion}
                onChange={(e) => onUpdateSwitches({ awsRegion: e.target.value })}
                className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
                placeholder="例如：us-east-1"
              />
            </div>
            <div>
              <label className="block text-xs text-gray-500 mb-1">
                AWS_ACCESS_KEY_ID（明文落库/live）
              </label>
              <input
                value={switches.awsAccessKeyId}
                onChange={(e) => onUpdateSwitches({ awsAccessKeyId: e.target.value })}
                className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
                placeholder="AKIA..."
              />
            </div>
            <div>
              <label className="block text-xs text-gray-500 mb-1">
                AWS_SECRET_ACCESS_KEY（明文落库/live）
              </label>
              <input
                type="password"
                value={switches.awsSecretAccessKey}
                onChange={(e) => onUpdateSwitches({ awsSecretAccessKey: e.target.value })}
                className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
                placeholder="明文存储，仅 UI 遮显"
              />
            </div>
            <p className="text-xs text-yellow-600 dark:text-yellow-400">
              ⚠ AWS 凭证按 ccs 做法明文随 env 落 provider.meta + live settings.json，
              不走加密（连接 token 仍加密）。仅本机 DB + live 文件暴露，不随导出泄漏。
            </p>
          </>
        )}
      </div>

      {/* 其它常见 CLAUDE_CODE_* 行为键 */}
      <div className="border-t border-gray-200 dark:border-gray-800 pt-3 space-y-2">
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={switches.disableNonessentialTraffic}
            onChange={(e) =>
              onUpdateSwitches({ disableNonessentialTraffic: e.target.checked })
            }
          />
          CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC
        </label>
        <label className="flex items-center gap-2 text-sm">
          <input
            type="checkbox"
            checked={switches.disableExperimentalBetas}
            onChange={(e) =>
              onUpdateSwitches({ disableExperimentalBetas: e.target.checked })
            }
          />
          CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS
        </label>
        <div>
          <label className="block text-xs text-gray-500 mb-1">
            CLAUDE_CODE_MAX_OUTPUT_TOKENS
          </label>
          <input
            value={switches.maxOutputTokens}
            onChange={(e) => onUpdateSwitches({ maxOutputTokens: e.target.value })}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-transparent font-mono"
            placeholder="例如：32000"
            inputMode="numeric"
          />
        </div>
      </div>

      {/* 裸 JSON 逃生舱 */}
      <div className="border-t border-gray-200 dark:border-gray-800 pt-3">
        <label className="block text-xs text-gray-500 mb-1">
          env 裸 JSON 逃生舱（meta.snapshot.env）
        </label>
        <textarea
          value={envRawText}
          onChange={(e) => onUpdateEnvRaw(e.target.value)}
          rows={5}
          className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-xs bg-transparent font-mono"
          placeholder='{"CUSTOM_KEY":"value"}'
        />
        {envRawError && <p className="text-xs text-red-500 mt-1">{envRawError}</p>}
        <p className="text-xs text-gray-500 mt-1">
          编辑未结构化的任意 env 键；结构化字段编辑会同步刷新此处。
        </p>
      </div>
    </div>
  );
}

interface ModelRoleRowProps {
  label: string;
  model: string;
  modelName: string;
  supportsOneM: boolean;
  onModelChange: (value: string) => void;
  onModelNameChange: (value: string) => void;
  onOneMChange: (enabled: boolean) => void;
}

function ModelRoleRow({
  label,
  model,
  modelName,
  supportsOneM,
  onModelChange,
  onModelNameChange,
  onOneMChange,
}: ModelRoleRowProps) {
  const usesOneM = supportsOneM && hasClaudeOneMMarker(model);
  return (
    <div className="grid grid-cols-[80px_1fr_1fr_60px] gap-2 items-center">
      <span className="text-xs text-gray-500">{label}</span>
      <input
        value={modelName}
        onChange={(e) => onModelNameChange(e.target.value)}
        className="px-2 py-1.5 border border-gray-300 dark:border-gray-700 rounded-md text-xs bg-transparent font-mono"
        placeholder="显示名"
      />
      <input
        value={stripClaudeOneMMarker(model)}
        onChange={(e) => onModelChange(e.target.value)}
        className="px-2 py-1.5 border border-gray-300 dark:border-gray-700 rounded-md text-xs bg-transparent font-mono"
        placeholder="实际请求模型 id"
      />
      {supportsOneM && (
        <label className="flex items-center gap-1 text-xs text-gray-500">
          <input
            type="checkbox"
            checked={usesOneM}
            onChange={(e) => onOneMChange(e.target.checked)}
          />
          1M
        </label>
      )}
    </div>
  );
}

// ── 合并结构化字段 + 裸 JSON 逃生舱 → meta ──────────────

/**
 * 把结构化字段序列化进 meta.snapshot.env，并保留裸 JSON 逃生舱内的非结构化键。
 *
 * 流程：以 initial.meta 为基底 → 用裸 JSON 文本覆盖 snapshot.env → 再用结构化字段覆盖
 * （结构化字段优先于裸 JSON，避免两边对同一键不一致时残留）。
 */
function mergeEnvIntoMeta(
  baseMeta: unknown,
  switches: ClaudeEnvSwitches,
  envRawText: string,
): unknown {
  // 1. 解析裸 JSON 逃生舱作为 snapshot.env 基底
  const rawEnv = parseRawEnvSafe(envRawText);

  // 2. 把裸 JSON 作为 snapshot.env 写入 meta 基底
  let root = asObject(baseMeta);
  if (!root) root = {};
  let snapshot = asObject(root.snapshot);
  if (!snapshot) snapshot = {};
  snapshot.env = rawEnv;
  root.snapshot = snapshot;

  // 3. 结构化字段覆盖（serializeClaudeEnv 会以 root.snapshot.env 为基底再叠加结构化键）
  return serializeClaudeEnv(root, switches);
}

function asObject(val: unknown): Record<string, unknown> | null {
  if (val && typeof val === 'object' && !Array.isArray(val)) {
    return val as Record<string, unknown>;
  }
  return null;
}

/**
 * 解析裸 JSON env 文本为对象；非法时返回空对象（不抛错，提交前由 envRawError 兜底）。
 */
function parseRawEnvSafe(text: string): Record<string, unknown> {
  try {
    const parsed = text.trim() === '' ? {} : JSON.parse(text);
    if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
      return parsed as Record<string, unknown>;
    }
  } catch {
    // 裸 JSON 非法：保留原样不合并，提交前会报错
  }
  return {};
}
