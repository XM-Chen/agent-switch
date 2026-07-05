import type { Provider } from '../../lib/api';
import { APP_TYPE_LABELS } from '../../lib/presentation';
import type { AppType } from '../../pages/providersUtils';
import { canMoveDown, canMoveUp } from '../../pages/providersUtils';
import { ProviderCard } from './ProviderCard';

interface AppTypeSectionProps {
  appType: AppType;
  /** 已按 sort_index 升序排好的 provider 列表。 */
  providers: Provider[];
  onSwitch: (id: string) => void;
  onEdit: (provider: Provider) => void;
  onDelete: (provider: Provider) => void;
  onMove: (appType: AppType, from: number, to: number) => void;
  /** 当前正在切换的 provider id（用于按钮 pending 态）。 */
  switchingId?: string | null;
  /** 排序请求是否在 pending（上移/下移按钮禁用）。 */
  movePending?: boolean;
}

/** 单个 app_type 分组：标题 + provider 卡片列表（上下移排序）。 */
export function AppTypeSection({
  appType,
  providers,
  onSwitch,
  onEdit,
  onDelete,
  onMove,
  switchingId,
  movePending = false,
}: AppTypeSectionProps) {
  const label = APP_TYPE_LABELS[appType] ?? appType;

  return (
    <section className="space-y-3">
      <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
        {label}
        <span className="ml-2 text-sm font-normal text-gray-400">
          {providers.length}
        </span>
      </h2>
      {providers.length === 0 ? (
        <p className="text-sm text-gray-400 py-4">
          暂无 {label} provider，点击右上角「添加 provider」创建。
        </p>
      ) : (
        <div className="space-y-2">
          {providers.map((p, index) => (
            <ProviderCard
              key={p.id}
              provider={p}
              canUp={canMoveUp(index)}
              canDown={canMoveDown(index, providers.length)}
              switchPending={switchingId === p.id}
              movePending={movePending}
              onSwitch={() => onSwitch(p.id)}
              onEdit={() => onEdit(p)}
              onDelete={() => onDelete(p)}
              onMoveUp={() => onMove(appType, index, index - 1)}
              onMoveDown={() => onMove(appType, index, index + 1)}
            />
          ))}
        </div>
      )}
    </section>
  );
}
