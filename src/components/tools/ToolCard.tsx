import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { toolsApi, type ToolBackup, type ToolStatus } from '../../lib/api';
import { CATEGORY_COLORS, CATEGORY_LABELS, TOOL_LABELS } from '../../lib/presentation';

interface ToolCardProps {
  tool: ToolStatus;
}

export function ToolCard({ tool }: ToolCardProps) {
  const queryClient = useQueryClient();

  const { data: backups = [] } = useQuery({
    queryKey: ['tools', 'backups', tool.tool],
    queryFn: () => toolsApi.backups(tool.tool),
    enabled: tool.enabled,
  });

  const toggle = useMutation({
    mutationFn: (enabled: boolean) => toolsApi.setTakeover(tool.tool, enabled),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['tools'] }),
    onError: (e: Error) => alert(`操作失败: ${e.message}`),
  });

  const handleToggle = () => {
    if (!toggle.isPending) {
      toggle.mutate(!tool.enabled);
    }
  };

  return (
    <div
      className={`rounded-lg border p-5 space-y-4 ${
        tool.supports_takeover
          ? 'bg-white dark:bg-gray-900 border-gray-200 dark:border-gray-800'
          : 'bg-gray-50 dark:bg-gray-900/50 border-gray-200 dark:border-gray-800'
      }`}
    >
      {/* 标题行 */}
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-3">
          <h3 className="font-semibold text-lg">{TOOL_LABELS[tool.tool] || tool.tool}</h3>
          <span
            className={`px-2 py-0.5 rounded text-xs font-medium ${CATEGORY_COLORS[tool.live_category] || CATEGORY_COLORS.unrecognized}`}
          >
            {CATEGORY_LABELS[tool.live_category] || tool.live_category}
          </span>
        </div>
        {tool.supports_takeover && (
          <button
            onClick={handleToggle}
            disabled={toggle.isPending}
            className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
              tool.enabled ? 'bg-green-600' : 'bg-gray-300 dark:bg-gray-700'
            }`}
          >
            <span
              className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                tool.enabled ? 'translate-x-6' : 'translate-x-1'
              }`}
            />
          </button>
        )}
      </div>

      {/* 状态信息 */}
      <div className="space-y-1.5 text-sm">
        {tool.supports_takeover && (
          <>
            <div className="flex justify-between">
              <span className="text-gray-500">接管状态</span>
              <span className={tool.enabled ? 'text-green-600' : 'text-gray-400'}>
                {tool.enabled ? '已开启' : '已关闭'}
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-500">目标地址</span>
              <span className="font-mono text-xs text-gray-600 dark:text-gray-400">
                {tool.last_target || '-'}
              </span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-500">最近写入</span>
              <span className="text-xs text-gray-500">{tool.last_applied_at || '从未'}</span>
            </div>
            {tool.last_error && (
              <div className="flex justify-between">
                <span className="text-gray-500">错误</span>
                <span className="text-xs text-red-500">{tool.last_error}</span>
              </div>
            )}
          </>
        )}
        {!tool.supports_takeover && (
          <p className="text-gray-500 text-xs">
            该工具暂不支持自动接管，可参考 OpenCode 手动配置方式接入。
          </p>
        )}
      </div>

      {/* 备份记录 */}
      {tool.supports_takeover && tool.enabled && backups.length > 0 && (
        <div className="border-t border-gray-200 dark:border-gray-800 pt-3 space-y-2">
          <h4 className="text-xs font-medium text-gray-500">备份记录</h4>
          <div className="space-y-1 max-h-32 overflow-y-auto">
            {backups.slice(0, 5).map((b: ToolBackup) => (
              <div key={b.id} className="text-xs text-gray-500 bg-gray-50 dark:bg-gray-800/50 rounded px-2 py-1">
                <p className="truncate">
                  {b.original_existed
                    ? `原文件: ${b.original_path}`
                    : '原文件不存在'}
                </p>
                {b.backup_path && <p className="truncate">备份至: {b.backup_path}</p>}
                <p className="text-gray-400">{b.created_at}</p>
              </div>
            ))}
          </div>
          <div className="text-xs text-gray-400 space-y-1">
            <p>恢复方法：找到对应备份文件，手动复制覆盖原配置目录下的同名文件即完成还原。</p>
            <p className="select-all font-mono text-gray-300">
              cp &quot;{backups[0]?.backup_path || '…'}&quot; &quot;{backups[0]?.original_path || '…'}&quot;
            </p>
          </div>
        </div>
      )}

      {/* 风险提示 */}
      {tool.supports_takeover && tool.enabled && (
        <p className="text-xs text-red-400 border-t border-gray-200 dark:border-gray-800 pt-3">
          注意：开启后将改写本机 {TOOL_LABELS[tool.tool]} 配置，使其指向 agent-switch 本地路由。
          关闭接管后配置不会自动还原，但备份文件保留供手动恢复。
        </p>
      )}
    </div>
  );
}
