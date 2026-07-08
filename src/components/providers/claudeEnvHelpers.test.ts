import { describe, expect, it } from 'vitest';
import {
  hasClaudeOneMMarker,
  parseClaudeEnv,
  serializeClaudeEnv,
  setClaudeOneMMarker,
  stripClaudeOneMMarker,
  validateApiTimeoutMs,
} from './claudeEnvHelpers';

describe('claudeEnvHelpers 1M marker', () => {
  it('detects [1M] suffix case-insensitively', () => {
    expect(hasClaudeOneMMarker('claude-3-5-sonnet[1M]')).toBe(true);
    expect(hasClaudeOneMMarker('claude-3-5-sonnet[1m]')).toBe(true);
    expect(hasClaudeOneMMarker('claude-3-5-sonnet')).toBe(false);
  });

  it('strips [1M] marker', () => {
    expect(stripClaudeOneMMarker('claude-3-5-sonnet[1M]')).toBe('claude-3-5-sonnet');
    expect(stripClaudeOneMMarker('claude-3-5-sonnet')).toBe('claude-3-5-sonnet');
  });

  it('sets/clears [1M] marker', () => {
    expect(setClaudeOneMMarker('claude-3-5-sonnet', true)).toBe('claude-3-5-sonnet[1M]');
    expect(setClaudeOneMMarker('claude-3-5-sonnet[1M]', false)).toBe('claude-3-5-sonnet');
    expect(setClaudeOneMMarker('', true)).toBe('');
  });
});

describe('validateApiTimeoutMs', () => {
  it('allows empty values so API_TIMEOUT_MS can be deleted', () => {
    expect(validateApiTimeoutMs('')).toBeNull();
    expect(validateApiTimeoutMs('   ')).toBeNull();
  });

  it('allows positive integer millisecond values', () => {
    expect(validateApiTimeoutMs('60000')).toBeNull();
    expect(validateApiTimeoutMs(' 120000 ')).toBeNull();
  });

  it('rejects non-integer or non-positive values', () => {
    expect(validateApiTimeoutMs('abc')).toContain('API_TIMEOUT_MS');
    expect(validateApiTimeoutMs('60s')).toContain('API_TIMEOUT_MS');
    expect(validateApiTimeoutMs('0')).toContain('API_TIMEOUT_MS');
    expect(validateApiTimeoutMs('-1')).toContain('API_TIMEOUT_MS');
    expect(validateApiTimeoutMs('1.5')).toContain('API_TIMEOUT_MS');
  });
});

describe('parseClaudeEnv / serializeClaudeEnv roundtrip', () => {
  it('parses known env keys into structured fields', () => {
    const meta = {
      snapshot: {
        env: {
          ANTHROPIC_DEFAULT_SONNET_MODEL: 'glm-4-plus[1M]',
          ANTHROPIC_DEFAULT_SONNET_MODEL_NAME: 'GLM 4 Plus',
          ANTHROPIC_DEFAULT_HAIKU_MODEL: 'glm-4-flash',
          ANTHROPIC_DEFAULT_OPUS_MODEL: 'glm-4-plus',
          ANTHROPIC_MODEL: 'deepseek-chat',
          API_TIMEOUT_MS: '60000',
          CLAUDE_CODE_USE_BEDROCK: '1',
          AWS_REGION: 'us-east-1',
          AWS_ACCESS_KEY_ID: 'AKIA',
          AWS_SECRET_ACCESS_KEY: 'secret',
          CLAUDE_CODE_DISABLE_NONESSENTIAL_TRAFFIC: '1',
          CUSTOM_KEY: 'keep-me',
        },
      },
    };
    const s = parseClaudeEnv(meta);
    expect(s.sonnetModel).toBe('glm-4-plus[1M]');
    expect(s.sonnetModelName).toBe('GLM 4 Plus');
    expect(s.haikuModel).toBe('glm-4-flash');
    expect(s.fallbackModel).toBe('deepseek-chat');
    expect(s.apiTimeout).toBe('60000');
    expect(s.useBedrock).toBe(true);
    expect(s.awsRegion).toBe('us-east-1');
    expect(s.disableNonessentialTraffic).toBe(true);
  });

  it('returns empty defaults for missing meta', () => {
    const s = parseClaudeEnv(null);
    expect(s.sonnetModel).toBe('');
    expect(s.useBedrock).toBe(false);
  });

  it('serialize preserves non-env snapshot keys + meta other keys', () => {
    const meta = {
      common_config_enabled: true,
      snapshot: {
        env: { CUSTOM_KEY: 'keep-me' },
        hooks: { a: 1 },
      },
    };
    const s = parseClaudeEnv(meta);
    s.sonnetModel = 'glm-4-plus';
    const out = serializeClaudeEnv(meta, s) as Record<string, unknown>;
    const outSnapshot = out.snapshot as Record<string, unknown>;
    const outEnv = outSnapshot.env as Record<string, unknown>;
    expect(outEnv.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe('glm-4-plus');
    expect(outEnv.CUSTOM_KEY).toBe('keep-me');
    expect((outSnapshot.hooks as Record<string, unknown>).a).toBe(1);
    expect(out.common_config_enabled).toBe(true);
  });

  it('empty values delete the env key', () => {
    const meta = { snapshot: { env: { ANTHROPIC_MODEL: 'old', KEEP: 'k' } } };
    const s = parseClaudeEnv(meta);
    s.fallbackModel = ''; // empty → delete
    const out = serializeClaudeEnv(meta, s) as Record<string, unknown>;
    const env = (out.snapshot as Record<string, unknown>).env as Record<string, unknown>;
    expect(env.ANTHROPIC_MODEL).toBeUndefined();
    expect(env.KEEP).toBe('k');
  });

  it('Bedrock disabled removes CLAUDE_CODE_USE_BEDROCK key', () => {
    const meta = { snapshot: { env: { CLAUDE_CODE_USE_BEDROCK: '1' } } };
    const s = parseClaudeEnv(meta);
    s.useBedrock = false;
    const out = serializeClaudeEnv(meta, s) as Record<string, unknown>;
    const env = (out.snapshot as Record<string, unknown>).env as Record<string, unknown>;
    expect(env.CLAUDE_CODE_USE_BEDROCK).toBeUndefined();
  });

  it('does not touch connection env keys (ANTHROPIC_BASE_URL/AUTH_TOKEN)', () => {
    // 前端编辑器不暴露连接键；strip_connection_env 后端会剥离，这里只验证 serialize 不会写入它们
    const meta = {
      snapshot: { env: { ANTHROPIC_BASE_URL: 'http://x', ANTHROPIC_AUTH_TOKEN: 'sk' } },
    };
    const s = parseClaudeEnv(meta);
    s.sonnetModel = 'new';
    const out = serializeClaudeEnv(meta, s) as Record<string, unknown>;
    const env = (out.snapshot as Record<string, unknown>).env as Record<string, unknown>;
    // 既有连接键保留（serialize 只动结构化字段，不删非结构化键）
    expect(env.ANTHROPIC_BASE_URL).toBe('http://x');
    expect(env.ANTHROPIC_AUTH_TOKEN).toBe('sk');
    expect(env.ANTHROPIC_DEFAULT_SONNET_MODEL).toBe('new');
  });
});

describe('applyPresetEnv', () => {
  it('merges preset env into snapshot.env, preserving existing non-preset keys', () => {
    // ponytail: 预设合并语义在 ProviderForm.handlePresetSelect 内联实现（基于 envRawText），
    // 这里用 serializeClaudeEnv + 手动合并验证底层契约。
    const meta = { snapshot: { env: { CUSTOM_KEY: 'keep' } } };
    const presetEnv = {
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'glm-4-plus',
      API_TIMEOUT_MS: '60000',
    };
    // 模拟 handlePresetSelect 的合并：rawBase + preset
    const rawBase = (meta.snapshot as Record<string, unknown>).env as Record<string, unknown>;
    const mergedEnv: Record<string, unknown> = { ...rawBase };
    for (const [k, v] of Object.entries(presetEnv)) {
      if (v.trim()) mergedEnv[k] = v;
      else delete mergedEnv[k];
    }
    const fakeMeta = { snapshot: { env: mergedEnv } };
    const s = parseClaudeEnv(fakeMeta);
    expect(s.sonnetModel).toBe('glm-4-plus');
    expect(s.apiTimeout).toBe('60000');
    expect((mergedEnv.CUSTOM_KEY)).toBe('keep');
  });
});
