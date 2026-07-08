/**
 * Claude Code env 行为开关解析/序列化 helper（对齐 ccs useModelState）。
 *
 * env 开关承载在 `provider.meta.snapshot.env`；本模块负责结构化字段 ↔ meta.snapshot.env 双向转换。
 */

/**
 * 1M 标记约定（对齐 ccs）：`[1M]` 后缀表示「声明支持 1M 上下文」。
 */
export const CLAUDE_ONE_M_MARKER = '[1M]';

export function hasClaudeOneMMarker(model: string): boolean {
  return model.trimEnd().toLowerCase().endsWith('[1m]');
}

export function stripClaudeOneMMarker(model: string): string {
  const trimmedEnd = model.trimEnd();
  if (!trimmedEnd.toLowerCase().endsWith('[1m]')) return model;
  return trimmedEnd.slice(0, -CLAUDE_ONE_M_MARKER.length).trimEnd();
}

export function setClaudeOneMMarker(model: string, enabled: boolean): string {
  const base = stripClaudeOneMMarker(model).trim();
  if (!base) return '';
  return enabled ? `${base}${CLAUDE_ONE_M_MARKER}` : base;
}

/**
 * API_TIMEOUT_MS：空串 = 删除该 env 键；非空须为正整数毫秒值。
 */
export function validateApiTimeoutMs(value: string): string | null {
  const trimmed = value.trim();
  if (!trimmed) return null;
  if (!/^[1-9]\d*$/.test(trimmed)) {
    return 'API_TIMEOUT_MS 须为正整数（毫秒），如 60000';
  }
  return null;
}

/**
 * 结构化 env 开关字段（对齐 ccs 已知键 + Bedrock 扩展）。
 */
export interface ClaudeEnvSwitches {
  // 模型三档（+ _NAME 显示名）
  haikuModel: string;
  haikuModelName: string;
  sonnetModel: string;
  sonnetModelName: string;
  opusModel: string;
  opusModelName: string;
  // 兜底模型
  fallbackModel: string;
  // 超时
  apiTimeout: string;
  // Bedrock 开关 + AWS 凭证（明文，用户拍板）
  useBedrock: boolean;
  awsRegion: string;
  awsAccessKeyId: string;
  awsSecretAccessKey: string;
  // 其它常见 CLAUDE_CODE_* 行为键
  disableNonessentialTraffic: boolean;
  maxOutputTokens: string;
  disableExperimentalBetas: boolean;
}

/**
 * 从 `meta.snapshot.env` 解析结构化字段。
 *
 * meta 缺失/非对象/无 snapshot 键 → 返回空默认值。
 */
export function parseClaudeEnv(meta: unknown): ClaudeEnvSwitches {
  const snapshot = getSnapshot(meta);
  const env = getEnv(snapshot);

  return {
    haikuModel: getString(env, 'ANTHROPIC_DEFAULT_HAIKU_MODEL'),
    haikuModelName: getString(env, 'ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME'),
    sonnetModel: getString(env, 'ANTHROPIC_DEFAULT_SONNET_MODEL'),
    sonnetModelName: getString(env, 'ANTHROPIC_DEFAULT_SONNET_MODEL_NAME'),
    opusModel: getString(env, 'ANTHROPIC_DEFAULT_OPUS_MODEL'),
    opusModelName: getString(env, 'ANTHROPIC_DEFAULT_OPUS_MODEL_NAME'),
    fallbackModel: getString(env, 'ANTHROPIC_MODEL'),
    apiTimeout: getString(env, 'API_TIMEOUT_MS'),
    useBedrock: getString(env, 'CLAUDE_CODE_USE_BEDROCK') === '1',
    awsRegion: getString(env, 'AWS_REGION'),
    awsAccessKeyId: getString(env, 'AWS_ACCESS_KEY_ID'),
    awsSecretAccessKey: getString(env, 'AWS_SECRET_ACCESS_KEY'),
    disableNonessentialTraffic: getString(env, 'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC') === '1',
    maxOutputTokens: getString(env, 'CLAUDE_CODE_MAX_OUTPUT_TOKENS'),
    disableExperimentalBetas: getString(env, 'CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS') === '1',
  };
}

/**
 * 把结构化字段写回 `meta.snapshot.env`，返回更新后的 meta 对象。
 *
 * 保留 meta 里的其它键（如 common_config_enabled）与 snapshot 内非 env 键（如 hooks）。
 * 空值字段 = 删除该 env 键（对齐 ccs）。
 */
export function serializeClaudeEnv(meta: unknown, switches: ClaudeEnvSwitches): unknown {
  // 1. 读取现有 meta/snapshot，保留非 env 部分
  let root = asObject(meta);
  if (!root) root = {};
  let snapshot = asObject(root.snapshot);
  if (!snapshot) snapshot = {};
  let env = asObject(snapshot.env);
  if (!env) env = {};

  // 2. 更新 env 键：空值删除，非空写入
  setOrDelete(env, 'ANTHROPIC_DEFAULT_HAIKU_MODEL', switches.haikuModel);
  setOrDelete(env, 'ANTHROPIC_DEFAULT_HAIKU_MODEL_NAME', switches.haikuModelName);
  setOrDelete(env, 'ANTHROPIC_DEFAULT_SONNET_MODEL', switches.sonnetModel);
  setOrDelete(env, 'ANTHROPIC_DEFAULT_SONNET_MODEL_NAME', switches.sonnetModelName);
  setOrDelete(env, 'ANTHROPIC_DEFAULT_OPUS_MODEL', switches.opusModel);
  setOrDelete(env, 'ANTHROPIC_DEFAULT_OPUS_MODEL_NAME', switches.opusModelName);
  setOrDelete(env, 'ANTHROPIC_MODEL', switches.fallbackModel);
  setOrDelete(env, 'API_TIMEOUT_MS', switches.apiTimeout);
  setOrDelete(env, 'CLAUDE_CODE_USE_BEDROCK', switches.useBedrock ? '1' : '');
  setOrDelete(env, 'AWS_REGION', switches.awsRegion);
  setOrDelete(env, 'AWS_ACCESS_KEY_ID', switches.awsAccessKeyId);
  setOrDelete(env, 'AWS_SECRET_ACCESS_KEY', switches.awsSecretAccessKey);
  setOrDelete(
    env,
    'CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC',
    switches.disableNonessentialTraffic ? '1' : '',
  );
  setOrDelete(env, 'CLAUDE_CODE_MAX_OUTPUT_TOKENS', switches.maxOutputTokens);
  setOrDelete(
    env,
    'CLAUDE_CODE_DISABLE_EXPERIMENTAL_BETAS',
    switches.disableExperimentalBetas ? '1' : '',
  );

  // 3. 组装回 meta
  snapshot.env = env;
  root.snapshot = snapshot;
  return root;
}

// ────────────────────────────────────────────────────────────
// 内部 helper
// ────────────────────────────────────────────────────────────

function asObject(val: unknown): Record<string, unknown> | null {
  if (val && typeof val === 'object' && !Array.isArray(val)) {
    return val as Record<string, unknown>;
  }
  return null;
}

function getSnapshot(meta: unknown): Record<string, unknown> {
  const root = asObject(meta);
  if (!root) return {};
  const snapshot = asObject(root.snapshot);
  return snapshot ?? {};
}

function getEnv(snapshot: Record<string, unknown>): Record<string, unknown> {
  const env = asObject(snapshot.env);
  return env ?? {};
}

function getString(obj: Record<string, unknown>, key: string): string {
  const val = obj[key];
  return typeof val === 'string' ? val : '';
}

function setOrDelete(obj: Record<string, unknown>, key: string, value: string) {
  const trimmed = value.trim();
  if (trimmed) {
    obj[key] = trimmed;
  } else {
    delete obj[key];
  }
}
