/**
 * 聚合模型页纯函数助手（C4）
 *
 * 只做数据变换与编解码，无 React / IPC 依赖，便于 vitest 覆盖。
 */

import type {
  AggregateRef,
  AggregateView,
  CustomAggregateView,
} from "@/lib/api/aggregation";

/** tier 选择器「未设置」的哨兵值（Radix Select 不允许空字符串 item value）。 */
export const TIER_NONE_VALUE = "__none__";

/**
 * 把聚合引用编码成 Select item 用的字符串值：`auto:<key>` / `custom:<id>`。
 *
 * 用「首个冒号」分隔：自动聚合 key（模型 id 原文，可能含 `/` 或 `.`，但当前不含
 * 冒号）与自定义聚合 id（uuid）都不会破坏解析。
 */
export function encodeAggregateRef(ref: AggregateRef): string {
  return `${ref.type}:${ref.value}`;
}

/** 解码 Select 值回聚合引用；未设置哨兵或非法输入返回 null。 */
export function decodeAggregateRef(value: string): AggregateRef | null {
  if (value === TIER_NONE_VALUE) return null;
  const idx = value.indexOf(":");
  if (idx === -1) return null;
  const type = value.slice(0, idx);
  const val = value.slice(idx + 1);
  if (type === "auto") return { type: "auto", value: val };
  if (type === "custom") return { type: "custom", value: val };
  return null;
}

/** 一个 tier 选择项。 */
export interface TierOption {
  /** 编码后的 Select item value。 */
  value: string;
  /** 展示标签（自动聚合用 key，自定义聚合用名称）。 */
  label: string;
  /** 该聚合当前候选是否为空（空聚合可选但需警示）。 */
  isEmpty: boolean;
}

/** 从自动聚合 + 自定义聚合构建 tier 下拉选项。 */
export function buildTierOptions(
  aggregates: AggregateView[],
  customAggregates: CustomAggregateView[],
): { auto: TierOption[]; custom: TierOption[] } {
  const auto: TierOption[] = aggregates.map((a) => ({
    value: encodeAggregateRef({ type: "auto", value: a.key }),
    label: a.key,
    isEmpty: a.members.length === 0,
  }));
  const custom: TierOption[] = customAggregates.map((c) => ({
    value: encodeAggregateRef({ type: "custom", value: c.id }),
    label: c.name,
    isEmpty: c.isEmpty,
  }));
  return { auto, custom };
}

/**
 * 把当前 tier 指向的聚合引用还原成 Select 的字符串值。
 *
 * 引用可能指向已消失的自动聚合 key（归零）或已删除的自定义聚合 id——此时仍编码原值，
 * 由调用方决定如何提示（Select 会显示为无匹配项的 raw 值）。
 */
export function tierRefToSelectValue(ref: AggregateRef | undefined): string {
  if (!ref) return TIER_NONE_VALUE;
  return encodeAggregateRef(ref);
}

/**
 * 描述一个聚合引用当前是否仍存在于可选项中（用于「指向已失效聚合」的提示）。
 *
 * 返回 `resolved` = true 表示引用能在当前自动/自定义聚合中找到。
 */
export function resolveTierRef(
  ref: AggregateRef | undefined,
  aggregates: AggregateView[],
  customAggregates: CustomAggregateView[],
): { resolved: boolean; label: string; isEmpty: boolean } | null {
  if (!ref) return null;
  if (ref.type === "auto") {
    const found = aggregates.find((a) => a.key === ref.value);
    return {
      resolved: !!found,
      label: ref.value,
      isEmpty: found ? found.members.length === 0 : true,
    };
  }
  const found = customAggregates.find((c) => c.id === ref.value);
  return {
    resolved: !!found,
    label: found ? found.name : ref.value,
    isEmpty: found ? found.isEmpty : true,
  };
}

/**
 * 毫秒 epoch → 本地日期时间字符串（用于展示上游最近刷新时间）。
 * 0 或非法值返回空串，由调用方决定显示占位。
 */
export function formatFetchedAt(ms: number | undefined): string {
  if (!ms || ms <= 0) return "";
  const date = new Date(ms);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString();
}

/** RFC3339 字符串 → 本地日期时间；空/非法返回空串。 */
export function formatRfc3339(value: string | undefined): string {
  if (!value) return "";
  const date = new Date(value);
  if (Number.isNaN(date.getTime())) return "";
  return date.toLocaleString();
}
