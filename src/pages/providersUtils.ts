/** ProvidersPage 专属纯函数：按 app_type 分组、排序与上下移计算。 */

import type { Provider } from '../lib/api';

/** 受支持的 app_type 列表（顺序决定页面分组的展示顺序）。 */
export const APP_TYPES = ['claude-code', 'codex'] as const;
export type AppType = (typeof APP_TYPES)[number];

/** 分组结果：按 app_type 分桶，每桶内按 sort_index 升序（NULLS last）。 */
export type ProvidersByAppType = Record<AppType, Provider[]>;

/**
 * 按 app_type 分组并按 sort_index 升序排序（NULLS last）。
 *
 * 非 `APP_TYPES` 列表内的 app_type 会被忽略，避免污染两个固定桶。
 */
export function groupByAppType(providers: Provider[]): ProvidersByAppType {
  const buckets: ProvidersByAppType = {
    'claude-code': [],
    codex: [],
  };
  for (const p of providers) {
    if (isAppType(p.app_type)) {
      buckets[p.app_type].push(p);
    }
  }
  buckets['claude-code'].sort(bySortIndexAscNullsLast);
  buckets.codex.sort(bySortIndexAscNullsLast);
  return buckets;
}

/** 边界判断：是否可上移（非首项）。 */
export const canMoveUp = (index: number): boolean => index > 0;

/** 边界判断：是否可下移（非末项）。 */
export const canMoveDown = (index: number, length: number): boolean =>
  index < length - 1;

/**
 * 计算移动后的 sort_index 数组（0 起连续重新编号），供 reorder 调用。
 *
 * 输入 `items` 应为已按当前 sort_index 排序的列表；返回新顺序下每项的
 * `{ id, sort_index }`，索引即新位置（0,1,2,...）。
 *
 * 越界的 `from`/`to` 会被夹紧到 `[0, length-1]`；`from===to` 时返回原顺序。
 */
export function moveItem(
  items: Provider[],
  from: number,
  to: number,
): { id: string; sort_index: number }[] {
  const n = items.length;
  if (n === 0) return [];
  const fromClamped = clamp(from, 0, n - 1);
  const toClamped = clamp(to, 0, n - 1);
  // 复制 id 序列并按 from→to 交换位置。
  const ids = items.map((p) => p.id);
  if (fromClamped === toClamped) {
    return ids.map((id, idx) => ({ id, sort_index: idx }));
  }
  const [moved] = ids.splice(fromClamped, 1);
  ids.splice(toClamped, 0, moved);
  return ids.map((id, idx) => ({ id, sort_index: idx }));
}

// ── 内部 helpers ──────────────────────────────────────────

function isAppType(value: string): value is AppType {
  return (APP_TYPES as readonly string[]).includes(value);
}

function bySortIndexAscNullsLast(a: Provider, b: Provider): number {
  const ai = a.sort_index;
  const bi = b.sort_index;
  if (ai === null && bi === null) return 0;
  if (ai === null) return 1; // NULLS last
  if (bi === null) return -1;
  return ai - bi;
}

function clamp(value: number, min: number, max: number): number {
  if (value < min) return min;
  if (value > max) return max;
  return value;
}
