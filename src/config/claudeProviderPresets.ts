/**
 * Claude Code provider 预设模板（对齐 ccs）。
 *
 * 每个预设 = env 行为开关的整套预填（不含连接层：base_url/token 走端点体系）。
 * 首批取 agent-switch 既有示例集（GLM/Kimi/MiniMax 等）+ Bedrock 作为最小代表集。
 */

export interface ClaudeProviderPreset {
  name: string;
  /** env 行为开关键值（不含 ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN 连接层）。 */
  env: Record<string, string>;
}

/**
 * Claude Code 聚合商 / Bedrock 预设清单。
 *
 * 参考 ccs `claudeProviderPresets.ts`，按 agent-switch 后端既有示例适配。
 * 模型名取常见默认值，用户可按需编辑。
 */
export const CLAUDE_PROVIDER_PRESETS: ClaudeProviderPreset[] = [
  {
    name: 'GLM（智谱清言）',
    env: {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'glm-4-flash',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'glm-4-plus',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'glm-4-plus',
      API_TIMEOUT_MS: '60000',
    },
  },
  {
    name: 'Kimi（月之暗面）',
    env: {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'moonshot-v1-8k',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'moonshot-v1-32k',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'moonshot-v1-128k',
      API_TIMEOUT_MS: '60000',
    },
  },
  {
    name: 'MiniMax',
    env: {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'abab6.5s-chat',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'abab6.5-chat',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'abab6.5-chat',
      API_TIMEOUT_MS: '60000',
    },
  },
  {
    name: 'DeepSeek',
    env: {
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'deepseek-chat',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'deepseek-chat',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'deepseek-chat',
      ANTHROPIC_MODEL: 'deepseek-chat',
      API_TIMEOUT_MS: '60000',
    },
  },
  {
    name: 'AWS Bedrock',
    env: {
      CLAUDE_CODE_USE_BEDROCK: '1',
      AWS_REGION: '',
      AWS_ACCESS_KEY_ID: '',
      AWS_SECRET_ACCESS_KEY: '',
      ANTHROPIC_DEFAULT_HAIKU_MODEL: 'claude-3-haiku-20240307',
      ANTHROPIC_DEFAULT_SONNET_MODEL: 'claude-3-5-sonnet-20241022',
      ANTHROPIC_DEFAULT_OPUS_MODEL: 'claude-3-opus-20240229',
      API_TIMEOUT_MS: '60000',
    },
  },
];
