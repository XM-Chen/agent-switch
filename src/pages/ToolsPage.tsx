import { useQuery } from '@tanstack/react-query';
import { toolsApi } from '../lib/api';
import { ToolCard } from '../components/tools/ToolCard';
import { OpenCodeCard } from '../components/tools/OpenCodeCard';

export function ToolsPage() {
  const { data: tools = [], isLoading, error } = useQuery({
    queryKey: ['tools'],
    queryFn: toolsApi.list,
  });

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">工具</h1>
        <p className="text-sm text-gray-500 mt-1">Claude Code、Codex 等 AI 编程工具的接管与配置</p>
      </div>

      {isLoading && <p className="text-gray-500">加载中...</p>}
      {error && <p className="text-red-500">加载失败: {error.message}</p>}

      <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-4">
        {tools.map((tool) => {
          if (tool.tool === 'opencode') {
            return <OpenCodeCard key={tool.tool} />;
          }
          return <ToolCard key={tool.tool} tool={tool} />;
        })}
      </div>
    </div>
  );
}
