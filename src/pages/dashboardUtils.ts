/** Dashboard 专属纯函数：端点健康聚合与故障转移跳数计算。 */

export interface HealthSource {
  enabled?: boolean;
  cooldown_until: string | null;
  last_failure_at: string | null;
  last_success_at: string | null;
}

export interface HealthAgg {
  normal: number;
  cooling: number;
  recentFailure: number;
  idle: number;
}

export type HealthBucket = 'normal' | 'cooling' | 'recent_failure' | 'idle';

/** 最近失败窗口：1 小时内视为「最近失败」。 */
export const RECENT_FAILURE_WINDOW_MS = 60 * 60 * 1000;

/**
 * 将端点/候选按冷却/最近失败/正常/待用分桶。
 *
 * 优先级：cooldown(未过) > 最近失败(1h 内) > 有最近成功 > 待用。
 */
export function bucketHealth(
  cooldownUntil: string | null,
  lastFailureAt: string | null,
  lastSuccessAt: string | null,
  now: number,
  recentFailureWindow: number = RECENT_FAILURE_WINDOW_MS,
): HealthBucket {
  if (cooldownUntil) {
    const cd = Date.parse(cooldownUntil);
    if (!Number.isNaN(cd) && cd > now) return 'cooling';
  }
  if (lastFailureAt) {
    const lf = Date.parse(lastFailureAt);
    if (!Number.isNaN(lf) && now - lf <= recentFailureWindow) {
      return 'recent_failure';
    }
  }
  if (lastSuccessAt) {
    const ls = Date.parse(lastSuccessAt);
    if (!Number.isNaN(ls)) return 'normal';
  }
  return 'idle';
}

/**
 * 聚合端点健康度。优先用 endpoints 列表；endpoints 为空时回退到 routes candidates。
 *
 * 注意：endpoints 为空但 routes 有候选时，仅用 candidates 兜底聚合（避免重复计数）。
 */
export function aggregateEndpointHealth(
  endpoints: HealthSource[],
  routes: { candidates: HealthSource[] }[],
  now: number = Date.now(),
): HealthAgg {
  let normal = 0;
  let cooling = 0;
  let recentFailure = 0;
  let idle = 0;

  const sources: HealthSource[] =
    endpoints.length > 0
      ? endpoints
      : routes.flatMap((r) => r.candidates);

  for (const s of sources) {
    const bucket = bucketHealth(
      s.cooldown_until,
      s.last_failure_at,
      s.last_success_at,
      now,
    );
    if (bucket === 'normal') normal++;
    else if (bucket === 'cooling') cooling++;
    else if (bucket === 'recent_failure') recentFailure++;
    else idle++;
  }

  return { normal, cooling, recentFailure, idle };
}

/**
 * 解析 fallback_chain（JSON 数组）取跳数；失败或空返回 0。
 *
 * fallback_chain 形如 `[{"endpoint_id":...,"model":...,"status":...,"error":...}]`。
 */
export function countFallbackHops(chain: string | null): number {
  if (!chain) return 0;
  try {
    const parsed = JSON.parse(chain);
    if (Array.isArray(parsed)) return parsed.length;
  } catch {
    // 解析失败忽略
  }
  return 0;
}
