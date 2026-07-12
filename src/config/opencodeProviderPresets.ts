import type { ProviderCategory, OpenCodeProviderConfig } from "../types";
import type { PresetTheme, TemplateValueConfig } from "./claudeProviderPresets";

export interface OpenCodeProviderPreset {
  name: string;
  nameKey?: string; // i18n key for localized display name
  websiteUrl: string;
  apiKeyUrl?: string;
  settingsConfig: OpenCodeProviderConfig;
  isOfficial?: boolean;
  isPartner?: boolean;
  primePartner?: boolean; // 置顶合作伙伴（顶级）：徽章显示为心形
  partnerPromotionKey?: string;
  category?: ProviderCategory;
  templateValues?: Record<string, TemplateValueConfig>;
  theme?: PresetTheme;
  icon?: string;
  iconColor?: string;
  isCustomTemplate?: boolean;
}

export const opencodeNpmPackages = [
  { value: "@ai-sdk/openai", label: "OpenAI Responses" },
  { value: "@ai-sdk/openai-compatible", label: "OpenAI Compatible" },
  { value: "@ai-sdk/anthropic", label: "Anthropic" },
  { value: "@ai-sdk/amazon-bedrock", label: "Amazon Bedrock" },
  { value: "@ai-sdk/google", label: "Google (Gemini)" },
] as const;

export interface PresetModelVariant {
  id: string;
  name?: string;
  contextLimit?: number;
  outputLimit?: number;
  modalities?: { input: string[]; output: string[] };
  options?: Record<string, unknown>;
  variants?: Record<string, Record<string, unknown>>;
}

export const OPENCODE_PRESET_MODEL_VARIANTS: Record<
  string,
  PresetModelVariant[]
> = {
  "@ai-sdk/openai-compatible": [
    {
      id: "MiniMax-M2.7",
      name: "MiniMax M2.7",
      contextLimit: 204800,
      outputLimit: 131072,
      modalities: { input: ["text"], output: ["text"] },
    },
    {
      id: "glm-5.1",
      name: "GLM 5.1",
      contextLimit: 204800,
      outputLimit: 131072,
      modalities: { input: ["text"], output: ["text"] },
    },
    {
      id: "kimi-k2.6",
      name: "Kimi K2.6",
      contextLimit: 262144,
      outputLimit: 262144,
      modalities: { input: ["text", "image", "video"], output: ["text"] },
    },
    {
      id: "step-3.5-flash-2603",
      name: "Step 3.5 Flash 2603",
      contextLimit: 262144,
    },
    {
      id: "step-3.5-flash",
      name: "Step 3.5 Flash",
      contextLimit: 262144,
    },
  ],
  "@ai-sdk/google": [
    {
      id: "gemini-2.5-flash-lite",
      name: "Gemini 2.5 Flash Lite",
      contextLimit: 1048576,
      outputLimit: 65536,
      modalities: {
        input: ["text", "image", "pdf", "video", "audio"],
        output: ["text"],
      },
      variants: {
        auto: {
          thinkingConfig: { includeThoughts: true, thinkingBudget: -1 },
        },
        "no-thinking": { thinkingConfig: { thinkingBudget: 0 } },
      },
    },
    {
      id: "gemini-3.5-flash",
      name: "Gemini 3.5 Flash",
      contextLimit: 1048576,
      outputLimit: 65536,
      modalities: {
        input: ["text", "image", "pdf", "video", "audio"],
        output: ["text"],
      },
      variants: {
        minimal: {
          thinkingConfig: { includeThoughts: true, thinkingLevel: "minimal" },
        },
        low: {
          thinkingConfig: { includeThoughts: true, thinkingLevel: "low" },
        },
        medium: {
          thinkingConfig: { includeThoughts: true, thinkingLevel: "medium" },
        },
        high: {
          thinkingConfig: { includeThoughts: true, thinkingLevel: "high" },
        },
      },
    },
  ],
  "@ai-sdk/openai": [
    {
      id: "gpt-5.5",
      name: "GPT-5.5",
      contextLimit: 400000,
      outputLimit: 128000,
      modalities: { input: ["text", "image"], output: ["text"] },
      variants: {
        low: {
          reasoningEffort: "low",
          reasoningSummary: "auto",
          textVerbosity: "medium",
        },
        medium: {
          reasoningEffort: "medium",
          reasoningSummary: "auto",
          textVerbosity: "medium",
        },
        high: {
          reasoningEffort: "high",
          reasoningSummary: "auto",
          textVerbosity: "medium",
        },
        xhigh: {
          reasoningEffort: "xhigh",
          reasoningSummary: "auto",
          textVerbosity: "medium",
        },
      },
    },
  ],
  "@ai-sdk/amazon-bedrock": [
    {
      id: "global.anthropic.claude-opus-4-8",
      name: "Claude Opus 4.8",
      contextLimit: 1000000,
      outputLimit: 128000,
      modalities: { input: ["text", "image", "pdf"], output: ["text"] },
    },
    {
      id: "global.anthropic.claude-sonnet-5",
      name: "Claude Sonnet 5",
      contextLimit: 1000000,
      outputLimit: 64000,
      modalities: { input: ["text", "image", "pdf"], output: ["text"] },
    },
    {
      id: "global.anthropic.claude-haiku-4-5-20251001-v1:0",
      name: "Claude Haiku 4.5",
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ["text", "image", "pdf"], output: ["text"] },
    },
    {
      id: "us.amazon.nova-pro-v1:0",
      name: "Amazon Nova Pro",
      contextLimit: 300000,
      outputLimit: 5000,
      modalities: { input: ["text", "image"], output: ["text"] },
    },
    {
      id: "us.meta.llama4-maverick-17b-instruct-v1:0",
      name: "Meta Llama 4 Maverick",
      contextLimit: 131072,
      outputLimit: 131072,
      modalities: { input: ["text"], output: ["text"] },
    },
    {
      id: "us.deepseek.r1-v1:0",
      name: "DeepSeek R1",
      contextLimit: 131072,
      outputLimit: 131072,
      modalities: { input: ["text"], output: ["text"] },
    },
  ],
  "@ai-sdk/anthropic": [
    {
      id: "claude-sonnet-4-5-20250929",
      name: "Claude Sonnet 4.5",
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ["text", "image", "pdf"], output: ["text"] },
      variants: {
        low: { effort: "low" },
        medium: { effort: "medium" },
        high: { effort: "high" },
      },
    },
    {
      id: "claude-opus-4-5-20251101",
      name: "Claude Opus 4.5",
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ["text", "image", "pdf"], output: ["text"] },
      variants: {
        low: { thinking: { budgetTokens: 5000, type: "enabled" } },
        medium: { thinking: { budgetTokens: 13000, type: "enabled" } },
        high: { thinking: { budgetTokens: 18000, type: "enabled" } },
      },
    },
    {
      id: "claude-opus-4-8",
      name: "Claude Opus 4.8",
      contextLimit: 1000000,
      outputLimit: 128000,
      modalities: { input: ["text", "image", "pdf"], output: ["text"] },
      variants: {
        low: { effort: "low" },
        medium: { effort: "medium" },
        high: { effort: "high" },
        max: { effort: "max" },
      },
    },
    {
      id: "claude-haiku-4-5-20251001",
      name: "Claude Haiku 4.5",
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ["text", "image", "pdf"], output: ["text"] },
    },
    {
      id: "gemini-claude-opus-4-5-thinking",
      name: "Antigravity - Claude Opus 4.5",
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ["text", "image", "pdf"], output: ["text"] },
      variants: {
        low: { effort: "low" },
        medium: { effort: "medium" },
        high: { effort: "high" },
      },
    },
    {
      id: "gemini-claude-sonnet-4-5-thinking",
      name: "Antigravity - Claude Sonnet 4.5",
      contextLimit: 200000,
      outputLimit: 64000,
      modalities: { input: ["text", "image", "pdf"], output: ["text"] },
      variants: {
        low: { thinking: { budgetTokens: 5000, type: "enabled" } },
        medium: { thinking: { budgetTokens: 13000, type: "enabled" } },
        high: { thinking: { budgetTokens: 18000, type: "enabled" } },
      },
    },
  ],
};

/**
 * Look up preset metadata for a model by npm package and model ID.
 * Returns enrichment fields (options, limit, modalities) that can be
 * merged into a model definition when the user's config doesn't already
 * provide them.
 */
export function getPresetModelDefaults(
  npm: string,
  modelId: string,
): PresetModelVariant | undefined {
  const models = OPENCODE_PRESET_MODEL_VARIANTS[npm];
  if (!models) return undefined;
  return models.find((m) => m.id === modelId);
}

export const opencodeProviderPresets: OpenCodeProviderPreset[] = [
  {
    name: "火山Agentplan",
    websiteUrl: "https://www.volcengine.com/product/ark",
    apiKeyUrl: "https://www.volcengine.com/activity/codingplan?ac=MMAP8JTTCAQ2",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "火山Agentplan",
      options: {
        baseURL: "https://ark.cn-beijing.volces.com/api/coding/v3",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "ark-code-latest": {
          name: "Ark Code Latest",
        },
      },
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
  },
  {
    name: "BytePlus",
    websiteUrl: "https://www.byteplus.com/en/product/modelark",
    apiKeyUrl: "https://www.byteplus.com/en/product/modelark",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "BytePlus",
      options: {
        baseURL: "https://ark.ap-southeast.bytepluses.com/api/coding/v3",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "ark-code-latest": {
          name: "Ark Code Latest",
        },
      },
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
  },
  {
    name: "DouBaoSeed",
    websiteUrl: "https://www.volcengine.com/product/doubao",
    apiKeyUrl:
      "https://console.volcengine.com/ark/region:ark+cn-beijing/apiKey?apikey=%7B%7D",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "DouBaoSeed",
      options: {
        baseURL: "https://ark.cn-beijing.volces.com/api/v3",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "doubao-seed-2-1-pro-260628": {
          name: "Doubao Seed 2.1 Pro",
        },
      },
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
  },
  {
    name: "DeepSeek",
    websiteUrl: "https://platform.deepseek.com",
    apiKeyUrl: "https://platform.deepseek.com/api_keys",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      options: {
        baseURL: "https://api.deepseek.com/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "deepseek-v4-pro": { name: "DeepSeek V4 Pro" },
        "deepseek-v4-flash": { name: "DeepSeek V4 Flash" },
      },
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
  },
  {
    name: "Zhipu GLM",
    websiteUrl: "https://open.bigmodel.cn",
    apiKeyUrl: "https://www.bigmodel.cn/claude-code",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "Zhipu GLM",
      options: {
        baseURL: "https://open.bigmodel.cn/api/coding/paas/v4",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "glm-5.1": { name: "GLM-5.1" },
      },
    },
    category: "cn_official",
    icon: "zhipu",
    iconColor: "#0F62FE",
    templateValues: {
      baseURL: {
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
  },
  {
    name: "Zhipu GLM en",
    websiteUrl: "https://z.ai",
    apiKeyUrl: "https://z.ai/subscribe",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "Zhipu GLM en",
      options: {
        baseURL: "https://api.z.ai/api/coding/paas/v4",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "glm-5.1": { name: "GLM-5.1" },
      },
    },
    category: "cn_official",
    icon: "zhipu",
    iconColor: "#0F62FE",
    templateValues: {
      baseURL: {
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
  },
  {
    name: "Bailian",
    websiteUrl: "https://bailian.console.aliyun.com",
    apiKeyUrl: "https://bailian.console.aliyun.com/#/api-key",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "Bailian",
      options: {
        baseURL: "https://dashscope.aliyuncs.com/compatible-mode/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {},
    },
    category: "cn_official",
    icon: "bailian",
    iconColor: "#624AFF",
    templateValues: {
      baseURL: {
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
  },
  {
    name: "Kimi",
    websiteUrl: "https://platform.kimi.com",
    apiKeyUrl: "https://platform.kimi.com/console/api-keys",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "Kimi",
      options: {
        baseURL: "https://api.moonshot.cn/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "kimi-k2.7-code": { name: "Kimi K2.7 Code" },
      },
    },
    category: "cn_official",
    icon: "kimi",
    iconColor: "#6366F1",
    templateValues: {
      baseURL: {
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
  },
  {
    name: "Kimi For Coding",
    websiteUrl: "https://www.kimi.com/code/",
    apiKeyUrl: "https://platform.kimi.com/console/api-keys",
    settingsConfig: {
      npm: "@ai-sdk/anthropic",
      name: "Kimi For Coding",
      options: {
        baseURL: "https://api.kimi.com/coding/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "kimi-for-coding": { name: "Kimi For Coding" },
      },
    },
    category: "cn_official",
    icon: "kimi",
    iconColor: "#6366F1",
    templateValues: {
      baseURL: {
        label: "Base URL",
        placeholder: "https://api.kimi.com/coding/v1",
        defaultValue: "https://api.kimi.com/coding/v1",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "sk-...",
        editorValue: "",
      },
    },
  },
  {
    name: "StepFun",
    websiteUrl: "https://platform.stepfun.com/step-plan",
    apiKeyUrl: "https://platform.stepfun.com/interface-key",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "StepFun",
      options: {
        baseURL: "https://api.stepfun.com/step_plan/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "step-3.5-flash-2603": { name: "Step 3.5 Flash 2603" },
        "step-3.5-flash": { name: "Step 3.5 Flash" },
      },
    },
    category: "cn_official",
    icon: "stepfun",
    iconColor: "#16D6D2",
    templateValues: {
      baseURL: {
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
  },
  {
    name: "StepFun en",
    websiteUrl: "https://platform.stepfun.ai/step-plan",
    apiKeyUrl: "https://platform.stepfun.ai/interface-key",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "StepFun en",
      options: {
        baseURL: "https://api.stepfun.ai/step_plan/v1",
        apiKey: "",
      },
      models: {
        "step-3.5-flash-2603": { name: "Step 3.5 Flash 2603" },
        "step-3.5-flash": { name: "Step 3.5 Flash" },
      },
    },
    category: "cn_official",
    icon: "stepfun",
    iconColor: "#16D6D2",
    templateValues: {
      baseURL: {
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
  },
  {
    name: "StepFun Step Plan",
    websiteUrl: "https://platform.stepfun.com/docs/zh/step-plan/overview",
    apiKeyUrl: "https://platform.stepfun.com/interface-key",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "StepFun Step Plan",
      options: {
        baseURL: "https://api.stepfun.com/step_plan/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "step-3.5-flash": { name: "Step 3.5 Flash" },
      },
    },
    category: "cn_official",
    icon: "stepfun",
    iconColor: "#005AFF",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "step-...",
        editorValue: "",
      },
    },
  },
  {
    name: "ModelScope",
    websiteUrl: "https://modelscope.cn",
    apiKeyUrl: "https://modelscope.cn/my/myaccesstoken",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "ModelScope",
      options: {
        baseURL: "https://api-inference.modelscope.cn/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "ZhipuAI/GLM-5.1": { name: "GLM-5.1" },
      },
    },
    category: "aggregator",
    icon: "modelscope",
    iconColor: "#624AFF",
    templateValues: {
      baseURL: {
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
  },
  {
    name: "KAT-Coder",
    websiteUrl: "https://console.streamlake.ai",
    apiKeyUrl: "https://console.streamlake.ai/console/api-key",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "KAT-Coder",
      options: {
        baseURL:
          "https://vanchin.streamlake.ai/api/gateway/v1/endpoints/${ENDPOINT_ID}/openai",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "KAT-Coder-Pro": { name: "KAT-Coder Pro" },
      },
    },
    category: "cn_official",
    templateValues: {
      baseURL: {
        label: "Base URL",
        placeholder:
          "https://vanchin.streamlake.ai/api/gateway/v1/endpoints/${ENDPOINT_ID}/openai",
        defaultValue:
          "https://vanchin.streamlake.ai/api/gateway/v1/endpoints/${ENDPOINT_ID}/openai",
        editorValue: "",
      },
      ENDPOINT_ID: {
        label: "Vanchin Endpoint ID",
        placeholder: "ep-xxx-xxx",
        defaultValue: "",
        editorValue: "",
      },
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
    icon: "catcoder",
  },
  {
    name: "Longcat",
    websiteUrl: "https://longcat.chat/platform",
    apiKeyUrl: "https://longcat.chat/platform/api_keys",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "Longcat",
      options: {
        baseURL: "https://api.longcat.chat/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "LongCat-Flash-Chat": { name: "LongCat Flash Chat" },
      },
    },
    category: "cn_official",
    icon: "longcat",
    iconColor: "#29E154",
    templateValues: {
      baseURL: {
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
  },
  {
    name: "MiniMax",
    websiteUrl: "https://platform.minimaxi.com",
    apiKeyUrl: "https://platform.minimaxi.com/subscribe/coding-plan",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "MiniMax",
      options: {
        baseURL: "https://api.minimaxi.com/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "MiniMax-M2.7": { name: "MiniMax M2.7" },
      },
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
  },
  {
    name: "MiniMax en",
    websiteUrl: "https://platform.minimax.io",
    apiKeyUrl: "https://platform.minimax.io/subscribe/coding-plan",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "MiniMax en",
      options: {
        baseURL: "https://api.minimax.io/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "MiniMax-M2.7": { name: "MiniMax M2.7" },
      },
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
  },
  {
    name: "BaiLing",
    websiteUrl: "https://alipaytbox.yuque.com/sxs0ba/ling/get_started",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "BaiLing",
      options: {
        baseURL: "https://api.tbox.cn/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "Ling-2.5-1T": { name: "Ling 2.5-1T" },
      },
    },
    category: "cn_official",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
  },
  {
    name: "Xiaomi MiMo",
    websiteUrl: "https://platform.xiaomimimo.com",
    apiKeyUrl: "https://platform.xiaomimimo.com/#/console/api-keys",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "Xiaomi MiMo",
      options: {
        baseURL: "https://api.xiaomimimo.com/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "mimo-v2.5-pro": {
          name: "MiMo V2.5 Pro",
          limit: { context: 1048576, output: 131072 },
          modalities: { input: ["text"], output: ["text"] },
        },
        "mimo-v2.5": {
          name: "MiMo V2.5",
          limit: { context: 1048576, output: 131072 },
          modalities: { input: ["text", "image"], output: ["text"] },
        },
      },
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
  },
  {
    name: "Xiaomi MiMo Token Plan (China)",
    websiteUrl: "https://platform.xiaomimimo.com/#/token-plan",
    apiKeyUrl: "https://platform.xiaomimimo.com/#/console/plan-manage",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "Xiaomi MiMo Token Plan (China)",
      options: {
        baseURL: "https://token-plan-cn.xiaomimimo.com/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "mimo-v2.5-pro": {
          name: "MiMo V2.5 Pro",
          limit: { context: 1048576, output: 131072 },
          modalities: { input: ["text"], output: ["text"] },
        },
        "mimo-v2.5": {
          name: "MiMo V2.5",
          limit: { context: 1048576, output: 131072 },
          modalities: { input: ["text", "image"], output: ["text"] },
        },
      },
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
  },

  {
    name: "OpenRouter",
    websiteUrl: "https://openrouter.ai",
    apiKeyUrl: "https://openrouter.ai/keys",
    settingsConfig: {
      npm: "@ai-sdk/anthropic",
      name: "OpenRouter",
      options: {
        baseURL: "https://openrouter.ai/api/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "anthropic/claude-sonnet-5": { name: "Claude Sonnet 5" },
        "anthropic/claude-opus-4.8": { name: "Claude Opus 4.8" },
      },
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
  },
  {
    name: "Nvidia",
    websiteUrl: "https://build.nvidia.com",
    apiKeyUrl: "https://build.nvidia.com/settings/api-keys",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      name: "Nvidia",
      options: {
        baseURL: "https://integrate.api.nvidia.com/v1",
        apiKey: "",
        setCacheKey: true,
      },
      models: {
        "moonshotai/kimi-k2.5": { name: "Kimi K2.5" },
      },
    },
    category: "aggregator",
    icon: "nvidia",
    iconColor: "#000000",
    templateValues: {
      apiKey: {
        label: "API Key",
        placeholder: "",
        editorValue: "",
      },
    },
  },
  {
    name: "AWS Bedrock",
    websiteUrl: "https://aws.amazon.com/bedrock/",
    settingsConfig: {
      npm: "@ai-sdk/amazon-bedrock",
      name: "AWS Bedrock",
      options: {
        region: "${region}",
        accessKeyId: "${accessKeyId}",
        secretAccessKey: "${secretAccessKey}",
        setCacheKey: true,
      },
      models: {
        "global.anthropic.claude-opus-4-8": { name: "Claude Opus 4.8" },
        "global.anthropic.claude-sonnet-5": {
          name: "Claude Sonnet 5",
        },
        "global.anthropic.claude-haiku-4-5-20251001-v1:0": {
          name: "Claude Haiku 4.5",
        },
        "us.amazon.nova-pro-v1:0": { name: "Amazon Nova Pro" },
        "us.meta.llama4-maverick-17b-instruct-v1:0": {
          name: "Meta Llama 4 Maverick",
        },
        "us.deepseek.r1-v1:0": { name: "DeepSeek R1" },
      },
    },
    category: "cloud_provider",
    icon: "aws",
    iconColor: "#FF9900",
    templateValues: {
      region: {
        label: "AWS Region",
        placeholder: "us-west-2",
        defaultValue: "us-west-2",
        editorValue: "us-west-2",
      },
      accessKeyId: {
        label: "Access Key ID",
        placeholder: "AKIA...",
        editorValue: "",
      },
      secretAccessKey: {
        label: "Secret Access Key",
        placeholder: "your-secret-key",
        editorValue: "",
      },
    },
  },
  {
    name: "OpenAI Compatible",
    websiteUrl: "",
    settingsConfig: {
      npm: "@ai-sdk/openai-compatible",
      options: {
        baseURL: "",
        apiKey: "",
        setCacheKey: true,
      },
      models: {},
    },
    category: "custom",
    isCustomTemplate: true,
    icon: "generic",
    iconColor: "#6B7280",
    templateValues: {
      baseURL: {
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

  {
    name: "Oh My OpenCode",
    websiteUrl: "https://github.com/code-yeongyu/oh-my-openagent",
    settingsConfig: {
      npm: "",
      options: {},
      models: {},
    },
    category: "omo" as ProviderCategory,
    icon: "opencode",
    iconColor: "#8B5CF6",
    isCustomTemplate: true,
  },
  {
    name: "Oh My OpenCode Slim",
    websiteUrl: "https://github.com/alvinunreal/oh-my-opencode-slim",
    settingsConfig: {
      npm: "",
      options: {},
      models: {},
    },
    category: "omo-slim" as ProviderCategory,
    icon: "opencode",
    iconColor: "#6366F1",
    isCustomTemplate: true,
  },
];
