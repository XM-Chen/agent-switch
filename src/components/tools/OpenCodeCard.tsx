export function OpenCodeCard() {
  const codeSnippet = `# agent-switch 手动配置（OpenCode）
export OPENAI_BASE_URL="http://127.0.0.1:42567/v1"
# 鉴权令牌使用占位符（本地服务暂不校验）
export OPENAI_API_KEY="agent-switch-managed"
`;

  const handleCopy = () => {
    navigator.clipboard.writeText(codeSnippet).catch(() => {
      // fallback
    });
  };

  return (
    <div className="rounded-lg border border-gray-200 dark:border-gray-800 bg-gray-50 dark:bg-gray-900/50 p-5 space-y-4">
      <div className="flex items-center justify-between">
        <h3 className="font-semibold text-lg">OpenCode</h3>
        <span className="px-2 py-0.5 rounded text-xs font-medium bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400">
          手动配置
        </span>
      </div>

      <p className="text-sm text-gray-500">
        OpenCode 暂不支持自动接管，但可通过以下环境变量手动配置指向 agent-switch：
      </p>

      <div className="relative">
        <pre className="bg-gray-800 text-green-300 rounded-md p-3 text-xs overflow-x-auto font-mono">
          {codeSnippet}
        </pre>
        <button
          onClick={handleCopy}
          className="absolute top-2 right-2 px-2 py-1 bg-gray-700 text-white rounded text-xs hover:bg-gray-600"
        >
          复制
        </button>
      </div>

      <p className="text-xs text-gray-400">
        配置后 OpenCode 的请求将通过 agent-switch 的 /v1 入口转发。
        请注意确保上游端点已配置并启用。
      </p>
    </div>
  );
}
