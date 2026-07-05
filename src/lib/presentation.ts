/** 工具标签映射（前端展示）。 */
export const TOOL_LABELS: Record<string, string> = {
  'claude-code': 'Claude Code',
  codex: 'Codex',
  opencode: 'OpenCode',
};

/** 工具接管状态分类标签。 */
export const CATEGORY_LABELS: Record<string, string> = {
  agent_switch: 'agent-switch',
  official: '官方',
  third_party: '第三方',
  unconfigured: '未配置',
  unrecognized: '无法识别',
};

/** 工具接管状态分类颜色。 */
export const CATEGORY_COLORS: Record<string, string> = {
  agent_switch: 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400',
  official: 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-400',
  third_party: 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400',
  unconfigured: 'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400',
  unrecognized: 'bg-red-100 text-red-700 dark:bg-red-900/30 dark:text-red-400',
};

/** 切换器 app_type 标签映射。opencode 后续再补。 */
export const APP_TYPE_LABELS: Record<string, string> = {
  'claude-code': 'Claude Code',
  codex: 'Codex',
};

/** Provider 模式标签：proxy=代理 / direct=直连。 */
export const MODE_LABELS: Record<string, string> = {
  proxy: '代理',
  direct: '直连',
};

/**
 * Provider 模式颜色：与 CATEGORY_COLORS 区分。
 *
 * proxy 用 indigo（蓝紫系，区别于 CATEGORY_COLORS.official 的 blue-100），
 * direct 用 purple，两者与现有 category 色不撞色。
 */
export const MODE_COLORS: Record<string, string> = {
  proxy: 'bg-indigo-100 text-indigo-700 dark:bg-indigo-900/30 dark:text-indigo-400',
  direct: 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-400',
};
