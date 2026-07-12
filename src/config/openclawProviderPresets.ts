/**
 * OpenClaw provider presets configuration
 * OpenClaw uses models.providers structure with custom provider configs
 */
import type {
  ProviderCategory,
  OpenClawProviderConfig,
  OpenClawDefaultModel,
} from "../types";
import type { PresetTheme, TemplateValueConfig } from "./claudeProviderPresets";

/** Suggested default model configuration for a preset */
export interface OpenClawSuggestedDefaults {
  /** Default model config to apply (agents.defaults.model) */
  model?: OpenClawDefaultModel;
  /** Model catalog entries to add (agents.defaults.models) */
  modelCatalog?: Record<string, { alias?: string }>;
}

export interface OpenClawProviderPreset {
  name: string;
  nameKey?: string; // i18n key for localized display name
  websiteUrl: string;
  apiKeyUrl?: string;
  /** OpenClaw settings_config structure */
  settingsConfig: OpenClawProviderConfig;
  isOfficial?: boolean;
  isPartner?: boolean;
  primePartner?: boolean; // 置顶合作伙伴（顶级）：徽章显示为心形
  partnerPromotionKey?: string;
  category?: ProviderCategory;
  /** Template variable definitions */
  templateValues?: Record<string, TemplateValueConfig>;
  /** Visual theme config */
  theme?: PresetTheme;
  /** Icon name */
  icon?: string;
  /** Icon color */
  iconColor?: string;
  /** Mark as custom template (for UI distinction) */
  isCustomTemplate?: boolean;
  /** Suggested default model configuration */
  suggestedDefaults?: OpenClawSuggestedDefaults;
}

function rebaseOpenClawModelRef(modelRef: string, providerKey: string): string {
  const slashIndex = modelRef.indexOf("/");
  return slashIndex === -1
    ? `${providerKey}/${modelRef}`
    : `${providerKey}${modelRef.slice(slashIndex)}`;
}

/**
 * OpenClaw default model refs are stored as "<provider-key>/<model-id>".
 * Presets carry stable built-in keys for display/tests, but the real key is
 * chosen in the add-provider form, so rewrite refs right before submission.
 */
export function rebaseOpenClawSuggestedDefaults(
  defaults: OpenClawSuggestedDefaults,
  providerKey: string,
): OpenClawSuggestedDefaults {
  const key = providerKey.trim();
  if (!key) return defaults;

  return {
    model: defaults.model
      ? {
          ...defaults.model,
          primary: rebaseOpenClawModelRef(defaults.model.primary, key),
          fallbacks: defaults.model.fallbacks?.map((modelRef) =>
            rebaseOpenClawModelRef(modelRef, key),
          ),
        }
      : undefined,
    modelCatalog: defaults.modelCatalog
      ? Object.fromEntries(
          Object.entries(defaults.modelCatalog).map(([modelRef, entry]) => [
            rebaseOpenClawModelRef(modelRef, key),
            entry,
          ]),
        )
      : undefined,
  };
}

/**
 * OpenClaw API protocol options
 * @see https://github.com/openclaw/openclaw/blob/main/docs/gateway/configuration.md
 */
export const openclawApiProtocols = [
  { value: "openai-completions", label: "OpenAI Completions" },
  { value: "openai-responses", label: "OpenAI Responses" },
  { value: "anthropic-messages", label: "Anthropic Messages" },
  { value: "google-generative-ai", label: "Google Generative AI" },
  { value: "bedrock-converse-stream", label: "AWS Bedrock" },
] as const;

/**
 * OpenClaw provider presets list
 */
export const openclawProviderPresets: OpenClawProviderPreset[] = [
  {
    name: "火山Agentplan",
    websiteUrl: "https://www.volcengine.com/product/ark",
    apiKeyUrl: "https://www.volcengine.com/activity/codingplan?ac=MMAP8JTTCAQ2",
    settingsConfig: {
      baseUrl: "https://ark.cn-beijing.volces.com/api/coding/v3",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "ark-code-latest",
          name: "Ark Code Latest",
          contextWindow: 256000,
        },
      ],
    },
    category: "cn_official",
    icon: "huoshan",
    iconColor: "#3370FF",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "ark_agentplan/ark-code-latest" },
      modelCatalog: {
        "ark_agentplan/ark-code-latest": { alias: "Ark Code" },
      },
    },
  },
  {
    name: "BytePlus",
    websiteUrl: "https://www.byteplus.com/en/product/modelark",
    apiKeyUrl: "https://www.byteplus.com/en/product/modelark",
    settingsConfig: {
      baseUrl: "https://ark.ap-southeast.bytepluses.com/api/coding/v3",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "ark-code-latest",
          name: "Ark Code Latest",
          contextWindow: 256000,
        },
      ],
    },
    category: "cn_official",
    icon: "byteplus",
    iconColor: "#3370FF",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "byteplus/ark-code-latest" },
      modelCatalog: {
        "byteplus/ark-code-latest": { alias: "Ark Code" },
      },
    },
  },
  {
    name: "DouBaoSeed",
    websiteUrl: "https://www.volcengine.com/product/doubao",
    apiKeyUrl:
      "https://console.volcengine.com/ark/region:ark+cn-beijing/apiKey",
    settingsConfig: {
      baseUrl: "https://ark.cn-beijing.volces.com/api/v3",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "doubao-seed-2-1-pro-260628",
          name: "DouBao Seed 2.1 Pro",
          contextWindow: 262144,
          cost: { input: 0.84, output: 4.2 },
        },
      ],
    },
    category: "cn_official",
    icon: "doubao",
    iconColor: "#3370FF",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "doubaoseed/doubao-seed-2-1-pro-260628" },
      modelCatalog: {
        "doubaoseed/doubao-seed-2-1-pro-260628": { alias: "DouBao" },
      },
    },
  },
  // ========== Chinese Officials ==========
  {
    name: "DeepSeek",
    websiteUrl: "https://platform.deepseek.com",
    apiKeyUrl: "https://platform.deepseek.com/api_keys",
    settingsConfig: {
      baseUrl: "https://api.deepseek.com/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "deepseek-v4-pro",
          name: "DeepSeek V4 Pro",
          contextWindow: 1000000,
          cost: { input: 1.68, output: 3.36 },
        },
        {
          id: "deepseek-v4-flash",
          name: "DeepSeek V4 Flash",
          contextWindow: 1000000,
          cost: { input: 0.14, output: 0.28 },
        },
      ],
    },
    category: "cn_official",
    icon: "deepseek",
    iconColor: "#1E88E5",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "sk-...",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: {
        primary: "deepseek/deepseek-v4-flash",
        fallbacks: ["deepseek/deepseek-v4-pro"],
      },
      modelCatalog: {
        "deepseek/deepseek-v4-flash": { alias: "Flash" },
        "deepseek/deepseek-v4-pro": { alias: "Pro" },
      },
    },
  },
  {
    name: "Zhipu GLM",
    websiteUrl: "https://open.bigmodel.cn",
    apiKeyUrl: "https://www.bigmodel.cn/claude-code",
    settingsConfig: {
      baseUrl: "https://open.bigmodel.cn/api/coding/paas/v4",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "glm-5.1",
          name: "GLM-5.1",
          contextWindow: 128000,
          cost: { input: 0.001, output: 0.001 },
        },
      ],
    },
    category: "cn_official",
    icon: "zhipu",
    iconColor: "#0F62FE",
    templateValues: {
      baseUrl: {
        label: "Base URL",
        placeholder: "https://open.bigmodel.cn/api/coding/paas/v4",
        defaultValue: "https://open.bigmodel.cn/api/coding/paas/v4",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "zhipu/glm-5.1" },
      modelCatalog: { "zhipu/glm-5.1": { alias: "GLM" } },
    },
  },
  {
    name: "Zhipu GLM en",
    websiteUrl: "https://z.ai",
    apiKeyUrl: "https://z.ai/subscribe",
    settingsConfig: {
      baseUrl: "https://api.z.ai/api/coding/paas/v4",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "glm-5.1",
          name: "GLM-5.1",
          contextWindow: 128000,
          cost: { input: 0.001, output: 0.001 },
        },
      ],
    },
    category: "cn_official",
    icon: "zhipu",
    iconColor: "#0F62FE",
    templateValues: {
      baseUrl: {
        label: "Base URL",
        placeholder: "https://api.z.ai/api/coding/paas/v4",
        defaultValue: "https://api.z.ai/api/coding/paas/v4",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "zhipu-en/glm-5.1" },
      modelCatalog: { "zhipu-en/glm-5.1": { alias: "GLM" } },
    },
  },
  {
    name: "Qwen Coder",
    websiteUrl: "https://bailian.console.aliyun.com",
    apiKeyUrl: "https://bailian.console.aliyun.com/#/api-key",
    settingsConfig: {
      baseUrl: "https://dashscope.aliyuncs.com/compatible-mode/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "qwen3.5-plus",
          name: "Qwen3.5 Plus",
          contextWindow: 32000,
          cost: { input: 0.002, output: 0.006 },
        },
      ],
    },
    category: "cn_official",
    icon: "qwen",
    iconColor: "#FF6A00",
    templateValues: {
      baseUrl: {
        label: "Base URL",
        placeholder: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        defaultValue: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "sk-...",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "qwen/qwen3.5-plus" },
      modelCatalog: { "qwen/qwen3.5-plus": { alias: "Qwen" } },
    },
  },
  {
    name: "Kimi",
    websiteUrl: "https://platform.kimi.com",
    apiKeyUrl: "https://platform.kimi.com/console/api-keys",
    settingsConfig: {
      baseUrl: "https://api.moonshot.cn/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "kimi-k2.7-code",
          name: "Kimi K2.7 Code",
          contextWindow: 262144,
          cost: { input: 0.002, output: 0.006 },
        },
      ],
    },
    category: "cn_official",
    icon: "kimi",
    iconColor: "#6366F1",
    templateValues: {
      baseUrl: {
        label: "Base URL",
        placeholder: "https://api.moonshot.cn/v1",
        defaultValue: "https://api.moonshot.cn/v1",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "sk-...",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "kimi/kimi-k2.7-code" },
      modelCatalog: { "kimi/kimi-k2.7-code": { alias: "Kimi" } },
    },
  },
  {
    name: "Kimi For Coding",
    websiteUrl: "https://www.kimi.com/code/",
    apiKeyUrl: "https://platform.kimi.com/console/api-keys",
    settingsConfig: {
      baseUrl: "https://api.kimi.com/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "kimi-for-coding",
          name: "Kimi For Coding",
          contextWindow: 131072,
          cost: { input: 0.002, output: 0.006 },
        },
      ],
    },
    category: "cn_official",
    icon: "kimi",
    iconColor: "#6366F1",
    templateValues: {
      baseUrl: {
        label: "Base URL",
        placeholder: "https://api.kimi.com/v1",
        defaultValue: "https://api.kimi.com/v1",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "sk-...",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "kimi-coding/kimi-for-coding" },
      modelCatalog: { "kimi-coding/kimi-for-coding": { alias: "Kimi" } },
    },
  },
  {
    name: "StepFun",
    websiteUrl: "https://platform.stepfun.com/step-plan",
    apiKeyUrl: "https://platform.stepfun.com/interface-key",
    settingsConfig: {
      baseUrl: "https://api.stepfun.com/step_plan/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "step-3.5-flash-2603",
          name: "Step 3.5 Flash 2603",
          contextWindow: 262144,
        },
        {
          id: "step-3.5-flash",
          name: "Step 3.5 Flash",
          contextWindow: 262144,
        },
      ],
    },
    category: "cn_official",
    icon: "stepfun",
    iconColor: "#16D6D2",
    templateValues: {
      baseUrl: {
        label: "Base URL",
        placeholder: "https://api.stepfun.com/step_plan/v1",
        defaultValue: "https://api.stepfun.com/step_plan/v1",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "step-...",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "stepfun/step-3.5-flash-2603" },
      modelCatalog: {
        "stepfun/step-3.5-flash-2603": { alias: "StepFun" },
        "stepfun/step-3.5-flash": { alias: "StepFun Flash" },
      },
    },
  },
  {
    name: "StepFun en",
    websiteUrl: "https://platform.stepfun.ai/step-plan",
    apiKeyUrl: "https://platform.stepfun.ai/interface-key",
    settingsConfig: {
      baseUrl: "https://api.stepfun.ai/step_plan/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "step-3.5-flash-2603",
          name: "Step 3.5 Flash 2603",
          contextWindow: 262144,
        },
        {
          id: "step-3.5-flash",
          name: "Step 3.5 Flash",
          contextWindow: 262144,
        },
      ],
    },
    category: "cn_official",
    icon: "stepfun",
    iconColor: "#16D6D2",
    templateValues: {
      baseUrl: {
        label: "Base URL",
        placeholder: "https://api.stepfun.ai/step_plan/v1",
        defaultValue: "https://api.stepfun.ai/step_plan/v1",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "step-...",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "stepfun-en/step-3.5-flash-2603" },
      modelCatalog: {
        "stepfun-en/step-3.5-flash-2603": { alias: "StepFun" },
        "stepfun-en/step-3.5-flash": { alias: "StepFun Flash" },
      },
    },
  },
  {
    name: "MiniMax",
    websiteUrl: "https://platform.minimaxi.com",
    apiKeyUrl: "https://platform.minimaxi.com/subscribe/coding-plan",
    settingsConfig: {
      baseUrl: "https://api.minimaxi.com/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "MiniMax-M2.7",
          name: "MiniMax M2.7",
          contextWindow: 200000,
          cost: { input: 0.001, output: 0.004 },
        },
      ],
    },
    category: "cn_official",
    theme: {
      backgroundColor: "#f64551",
      textColor: "#FFFFFF",
    },
    icon: "minimax",
    iconColor: "#FF6B6B",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "minimax/MiniMax-M2.7" },
      modelCatalog: { "minimax/MiniMax-M2.7": { alias: "MiniMax" } },
    },
  },
  {
    name: "MiniMax en",
    websiteUrl: "https://platform.minimax.io",
    apiKeyUrl: "https://platform.minimax.io/subscribe/coding-plan",
    settingsConfig: {
      baseUrl: "https://api.minimax.io/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "MiniMax-M2.7",
          name: "MiniMax M2.7",
          contextWindow: 200000,
          cost: { input: 0.001, output: 0.004 },
        },
      ],
    },
    category: "cn_official",
    theme: {
      backgroundColor: "#f64551",
      textColor: "#FFFFFF",
    },
    icon: "minimax",
    iconColor: "#FF6B6B",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "minimax-en/MiniMax-M2.7" },
      modelCatalog: { "minimax-en/MiniMax-M2.7": { alias: "MiniMax" } },
    },
  },
  {
    name: "KAT-Coder",
    websiteUrl: "https://console.streamlake.ai",
    apiKeyUrl: "https://console.streamlake.ai/console/api-key",
    settingsConfig: {
      baseUrl:
        "https://vanchin.streamlake.ai/api/gateway/v1/endpoints/${ENDPOINT_ID}/openai",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "KAT-Coder-Pro",
          name: "KAT-Coder Pro",
          contextWindow: 128000,
          cost: { input: 0.002, output: 0.006 },
        },
      ],
    },
    category: "cn_official",
    icon: "catcoder",
    templateValues: {
      baseUrl: {
        label: "Base URL",
        placeholder:
          "https://vanchin.streamlake.ai/api/gateway/v1/endpoints/${ENDPOINT_ID}/openai",
        defaultValue:
          "https://vanchin.streamlake.ai/api/gateway/v1/endpoints/${ENDPOINT_ID}/openai",
        editorValue: "",
      },
      ENDPOINT_ID: {
        label: "Endpoint ID",
        placeholder: "",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "katcoder/KAT-Coder-Pro" },
      modelCatalog: { "katcoder/KAT-Coder-Pro": { alias: "KAT-Coder" } },
    },
  },
  {
    name: "Longcat",
    websiteUrl: "https://longcat.chat/platform",
    apiKeyUrl: "https://longcat.chat/platform/api_keys",
    settingsConfig: {
      baseUrl: "https://api.longcat.chat/v1",
      apiKey: "",
      api: "openai-completions",
      authHeader: true,
      models: [
        {
          id: "LongCat-Flash-Chat",
          name: "LongCat Flash Chat",
          contextWindow: 128000,
          cost: { input: 0.001, output: 0.004 },
        },
      ],
    },
    category: "cn_official",
    icon: "longcat",
    iconColor: "#29E154",
    templateValues: {
      baseUrl: {
        label: "Base URL",
        placeholder: "https://api.longcat.chat/v1",
        defaultValue: "https://api.longcat.chat/v1",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "longcat/LongCat-Flash-Chat" },
      modelCatalog: { "longcat/LongCat-Flash-Chat": { alias: "LongCat" } },
    },
  },
  {
    name: "BaiLing",
    websiteUrl: "https://alipaytbox.yuque.com/sxs0ba/ling/get_started",
    settingsConfig: {
      baseUrl: "https://api.tbox.cn/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "Ling-2.5-1T",
          name: "Ling 2.5 1T",
          contextWindow: 128000,
          cost: { input: 0.001, output: 0.004 },
        },
      ],
    },
    category: "cn_official",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "bailing/Ling-2.5-1T" },
      modelCatalog: { "bailing/Ling-2.5-1T": { alias: "BaiLing" } },
    },
  },
  {
    name: "Xiaomi MiMo",
    websiteUrl: "https://platform.xiaomimimo.com",
    apiKeyUrl: "https://platform.xiaomimimo.com/#/console/api-keys",
    settingsConfig: {
      baseUrl: "https://api.xiaomimimo.com/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "mimo-v2.5-pro",
          name: "MiMo V2.5 Pro",
          reasoning: true,
          input: ["text"],
          contextWindow: 1048576,
          maxTokens: 131072,
          cost: { input: 1, output: 3, cacheRead: 0.2, cacheWrite: 0 },
        },
      ],
    },
    category: "cn_official",
    icon: "xiaomimimo",
    iconColor: "#000000",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "xiaomimimo/mimo-v2.5-pro" },
      modelCatalog: { "xiaomimimo/mimo-v2.5-pro": { alias: "MiMo" } },
    },
  },
  {
    name: "Xiaomi MiMo Token Plan (China)",
    websiteUrl: "https://platform.xiaomimimo.com/#/token-plan",
    apiKeyUrl: "https://platform.xiaomimimo.com/#/console/plan-manage",
    settingsConfig: {
      baseUrl: "https://token-plan-cn.xiaomimimo.com/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "mimo-v2.5-pro",
          name: "MiMo V2.5 Pro",
          reasoning: true,
          input: ["text"],
          contextWindow: 1048576,
          maxTokens: 131072,
        },
        {
          id: "mimo-v2.5",
          name: "MiMo V2.5",
          reasoning: true,
          input: ["text", "image"],
          contextWindow: 1048576,
          maxTokens: 131072,
        },
      ],
    },
    category: "cn_official",
    icon: "xiaomimimo",
    iconColor: "#000000",
    templateValues: {
      apiKey: {
        label: "Token Plan API Key",
        placeholder: "tp-...",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "xiaomi-mimo-token-plan/mimo-v2.5-pro" },
      modelCatalog: {
        "xiaomi-mimo-token-plan/mimo-v2.5-pro": {
          alias: "MiMo Token Plan (China)",
        },
        "xiaomi-mimo-token-plan/mimo-v2.5": {
          alias: "MiMo Token Plan (China) Multimodal",
        },
      },
    },
  },

  // ========== Aggregators ==========
  {
    name: "OpenRouter",
    websiteUrl: "https://openrouter.ai",
    apiKeyUrl: "https://openrouter.ai/keys",
    settingsConfig: {
      baseUrl: "https://openrouter.ai/api/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "anthropic/claude-opus-4.8",
          name: "Claude Opus 4.8",
          contextWindow: 1000000,
          cost: { input: 5, output: 25 },
        },
        {
          id: "anthropic/claude-sonnet-5",
          name: "Claude Sonnet 5",
          contextWindow: 1000000,
          cost: { input: 3, output: 15 },
        },
      ],
    },
    category: "aggregator",
    icon: "openrouter",
    iconColor: "#6566F1",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "sk-or-...",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: {
        primary: "openrouter/anthropic/claude-opus-4.8",
        fallbacks: ["openrouter/anthropic/claude-sonnet-5"],
      },
      modelCatalog: {
        "openrouter/anthropic/claude-opus-4.8": { alias: "Opus" },
        "openrouter/anthropic/claude-sonnet-5": { alias: "Sonnet" },
      },
    },
  },
  {
    name: "ModelScope",
    websiteUrl: "https://modelscope.cn",
    apiKeyUrl: "https://modelscope.cn/my/myaccesstoken",
    settingsConfig: {
      baseUrl: "https://api-inference.modelscope.cn/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "ZhipuAI/GLM-5.1",
          name: "GLM-5.1",
          contextWindow: 128000,
          cost: { input: 0.001, output: 0.001 },
        },
      ],
    },
    category: "aggregator",
    icon: "modelscope",
    iconColor: "#624AFF",
    templateValues: {
      baseUrl: {
        label: "Base URL",
        placeholder: "https://api-inference.modelscope.cn/v1",
        defaultValue: "https://api-inference.modelscope.cn/v1",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "modelscope/ZhipuAI/GLM-5.1" },
      modelCatalog: { "modelscope/ZhipuAI/GLM-5.1": { alias: "GLM" } },
    },
  },
  {
    name: "SiliconFlow",
    websiteUrl: "https://siliconflow.cn",
    apiKeyUrl: "https://cloud.siliconflow.cn",
    settingsConfig: {
      baseUrl: "https://api.siliconflow.cn/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "Pro/MiniMaxAI/MiniMax-M2.7",
          name: "MiniMax M2.7",
          contextWindow: 200000,
          cost: { input: 0.001, output: 0.004 },
        },
      ],
    },
    category: "aggregator",
    icon: "siliconflow",
    iconColor: "#6E29F6",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "sk-...",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "siliconflow/Pro/MiniMaxAI/MiniMax-M2.7" },
      modelCatalog: {
        "siliconflow/Pro/MiniMaxAI/MiniMax-M2.7": { alias: "MiniMax" },
      },
    },
  },
  {
    name: "SiliconFlow en",
    websiteUrl: "https://siliconflow.com",
    apiKeyUrl: "https://cloud.siliconflow.cn",
    settingsConfig: {
      baseUrl: "https://api.siliconflow.com/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "MiniMaxAI/MiniMax-M2.7",
          name: "MiniMax M2.7",
          contextWindow: 200000,
          cost: { input: 0.001, output: 0.004 },
        },
      ],
    },
    category: "aggregator",
    icon: "siliconflow",
    iconColor: "#000000",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "sk-...",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "siliconflow-en/MiniMaxAI/MiniMax-M2.7" },
      modelCatalog: {
        "siliconflow-en/MiniMaxAI/MiniMax-M2.7": { alias: "MiniMax" },
      },
    },
  },
  {
    name: "Nvidia",
    websiteUrl: "https://build.nvidia.com",
    apiKeyUrl: "https://build.nvidia.com/settings/api-keys",
    settingsConfig: {
      baseUrl: "https://integrate.api.nvidia.com/v1",
      apiKey: "",
      api: "openai-completions",
      models: [
        {
          id: "moonshotai/kimi-k2.5",
          name: "Kimi K2.5",
          contextWindow: 131072,
          cost: { input: 0.002, output: 0.006 },
        },
      ],
    },
    category: "aggregator",
    icon: "nvidia",
    iconColor: "#000000",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "nvapi-...",
        editorValue: "",
      },
    },
    suggestedDefaults: {
      model: { primary: "nvidia/moonshotai/kimi-k2.5" },
      modelCatalog: { "nvidia/moonshotai/kimi-k2.5": { alias: "Kimi" } },
    },
  },

  // ========== Third Party Partners ==========
  // ========== Cloud Providers ==========
  {
    name: "AWS Bedrock",
    websiteUrl: "https://aws.amazon.com/bedrock/",
    settingsConfig: {
      // 请将 us-west-2 替换为你的 AWS Region
      baseUrl: "https://bedrock-runtime.us-west-2.amazonaws.com",
      apiKey: "",
      api: "bedrock-converse-stream",
      models: [
        {
          id: "anthropic.claude-opus-4-8",
          name: "Claude Opus 4.8",
          contextWindow: 1000000,
          cost: { input: 5, output: 25, cacheRead: 0.5, cacheWrite: 6.25 },
        },
        {
          id: "anthropic.claude-sonnet-5",
          name: "Claude Sonnet 5",
          contextWindow: 1000000,
          cost: { input: 3, output: 15, cacheRead: 0.3, cacheWrite: 3.75 },
        },
        {
          id: "anthropic.claude-haiku-4-5-20251022-v1:0",
          name: "Claude Haiku 4.5",
          contextWindow: 200000,
          cost: { input: 0.8, output: 4, cacheRead: 0.08, cacheWrite: 1 },
        },
      ],
    },
    category: "cloud_provider",
    icon: "aws",
    iconColor: "#FF9900",
  },

  // ========== Custom Template ==========
  {
    name: "OpenAI Compatible",
    websiteUrl: "",
    settingsConfig: {
      baseUrl: "",
      apiKey: "",
      api: "openai-completions",
      models: [],
    },
    category: "custom",
    isCustomTemplate: true,
    icon: "generic",
    iconColor: "#6B7280",
    templateValues: {
      baseUrl: {
        label: "Base URL",
        placeholder: "https://api.example.com/v1",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
  },
];
