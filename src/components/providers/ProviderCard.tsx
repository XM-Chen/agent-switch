import type { Provider } from '../../lib/api';
import {
  CATEGORY_COLORS,
  CATEGORY_LABELS,
  MODE_COLORS,
  MODE_LABELS,
} from '../../lib/presentation';

interface ProviderCardProps {
  provider: Provider;
  canUp: boolean;
  canDown: boolean;
  onSwitch: () => void;
  onEdit: () => void;
  onDelete: () => void;
  onMoveUp: () => void;
  onMoveDown: () => void;
  /** 切换按钮是否在 pending（避免并发点击）。 */
  switchPending?: boolean;
  /** 排序请求是否在 pending（上移/下移按钮禁用，避免并发 reorder）。 */
  movePending?: boolean;
}

/**
 * 单张 provider 卡片：名称 + 分类 badge + 模式标签 + 激活态高亮 + 操作。
 *
 * 激活态（is_current）通过左侧色条 + 边框 + 底色区分；切换按钮在已激活时
 * 显示「已激活」且禁用，避免重复切换。
 */
export function ProviderCard({
  provider,
  canUp,
  canDown,
  onSwitch,
  onEdit,
  onDelete,
  onMoveUp,
  onMoveDown,
  switchPending = false,
  movePending = false,
}: ProviderCardProps) {
  const categoryLabel = provider.category
    ? CATEGORY_LABELS[provider.category] ?? provider.category
    : null;
  const categoryColor = provider.category
    ? CATEGORY_COLORS[provider.category] ?? 'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400'
    : '';
  const modeLabel = MODE_LABELS[provider.mode] ?? provider.mode;
  const modeColor = MODE_COLORS[provider.mode] ?? '';

  return (
    <div
      className={`rounded-lg border p-4 transition-colors ${
        provider.is_current
          ? 'border-blue-500 dark:border-blue-400 bg-blue-50 dark:bg-blue-900/20'
          : 'border-gray-200 dark:border-gray-800 bg-white dark:bg-gray-900'
      }`}
    >
      <div className="flex items-start justify-between gap-3">
        <div className="min-w-0 flex-1">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="font-medium text-gray-900 dark:text-gray-100 truncate">
              {provider.name}
            </span>
            {provider.is_current && (
              <span className="px-2 py-0.5 rounded text-xs bg-blue-600 text-white">
                已激活
              </span>
            )}
            {categoryLabel && (
              <span
                className={`px-2 py-0.5 rounded text-xs ${categoryColor}`}
              >
                {categoryLabel}
              </span>
            )}
            <span className={`px-2 py-0.5 rounded text-xs ${modeColor}`}>
              {modeLabel}
            </span>
          </div>
          {provider.notes && (
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-1 line-clamp-2">
              {provider.notes}
            </p>
          )}
        </div>

        <div className="flex flex-col gap-1 shrink-0">
          <button
            type="button"
            onClick={onSwitch}
            disabled={provider.is_current || switchPending}
            className="px-3 py-1 rounded text-xs bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50 disabled:cursor-not-allowed"
          >
            {switchPending ? '切换中...' : provider.is_current ? '已激活' : '切换'}
          </button>
          <div className="flex gap-1">
            <button
              type="button"
              onClick={onMoveUp}
              disabled={!canUp || movePending}
              aria-label="上移"
              className="flex-1 px-2 py-1 rounded text-xs bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700 disabled:opacity-40 disabled:cursor-not-allowed"
            >
              ↑
            </button>
            <button
              type="button"
              onClick={onMoveDown}
              disabled={!canDown || movePending}
              aria-label="下移"
              className="flex-1 px-2 py-1 rounded text-xs bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700 disabled:opacity-40 disabled:cursor-not-allowed"
            >
              ↓
            </button>
          </div>
          <div className="flex gap-1">
            <button
              type="button"
              onClick={onEdit}
              className="flex-1 px-2 py-1 rounded text-xs text-gray-600 dark:text-gray-300 hover:bg-gray-100 dark:hover:bg-gray-800"
            >
              编辑
            </button>
            <button
              type="button"
              onClick={onDelete}
              className="flex-1 px-2 py-1 rounded text-xs text-red-500 hover:text-red-600"
            >
              删除
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
