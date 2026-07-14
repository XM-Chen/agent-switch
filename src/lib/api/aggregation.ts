import { invoke } from "@tauri-apps/api/core";
import type { AppId } from "./types";

// ============================================================================
// 类型（对齐 C1/C2 Rust serde 输出，camelCase）
//
// 后端命令均以 `#[tauri::command(rename_all = "camelCase")]` 注册，因此 invoke
// 用 snake_case 命令名 + camelCase 参数键。所有命令参数键统一 `{ appType }`（D11）。
// ============================================================================

/** 模型来源标记。 */
export type ModelSource = "fetched" | "manual";

/** 缓存的上游模型行（C1，`list_provider_models` 返回项）。 */
export interface ProviderModel {
  providerId: string;
  appType: string;
  /** 上游返回的 model id 原文，不做归一化改写。 */
  modelId: string;
  source: ModelSource;
  /** 后端 `skip_serializing_if = Option::is_none`，可能缺省。 */
  ownedBy?: string;
  /** 毫秒 epoch。 */
  fetchedAt: number;
}

/** 队列全量刷新汇总（C1，`refresh_provider_models_now` 返回）。 */
export interface RefreshSummary {
  refreshed: number;
  skipped: number;
  totalModels: number;
}

/** 单个上游的缓存状态（C1，`ModelCacheStatus.providers` 项）。 */
export interface ProviderCacheStatus {
  providerId: string;
  modelCount: number;
  /** 该上游任意缓存行中最新的 fetchedAt（毫秒 epoch）。 */
  latestFetchedAt: number;
}

/** 模型缓存整体状态（C1，`get_model_cache_status` 返回）。 */
export interface ModelCacheStatus {
  /** 每日全量刷新上次执行时刻（RFC3339），从未执行为缺省。 */
  lastFullRefresh?: string;
  providers: ProviderCacheStatus[];
}

/** 一个聚合内的一个上游候选（C2）。 */
export interface AggregateMember {
  providerId: string;
  providerName: string;
  /** 上游返回的 model id 原文（路由改写精确透传）。 */
  modelId: string;
  source: ModelSource;
}

/** 一个自动聚合视图（C2，`get_aggregates` 返回项）。 */
export interface AggregateView {
  /** 聚合 key = 桶内首次出现的 model_id 原文（display_id，保留大小写）。 */
  key: string;
  /** 已按故障转移队列序 P1→P2 排列的上游候选。 */
  members: AggregateMember[];
}

/** 一个自定义聚合的派生视图（C2，`get_custom_aggregates` 返回项）。 */
export interface CustomAggregateView {
  id: string;
  name: string;
  /**
   * 原始成员 key 列表（自动聚合 key），直接透传 ordered_members，含已归零成员，
   * 顺序即用户保存时的顺序。编辑对话框用它精确回填（无需从展平候选近似重建）。
   */
  orderedMembers: string[];
  /** 展平后的候选，按「外层成员序 × 内层上游序」排列并去重（D7）。 */
  members: AggregateMember[];
  /** 归零标记（D7：只标记不删）。全部成员归零 → true。 */
  isEmpty: boolean;
  /** ordered_members 中当前已归零/不存在的自动聚合 key（原样保留用户输入）。 */
  missingMembers: string[];
}

/** 聚合引用（tagged enum，D7）。指自动聚合 key 或自定义聚合 id。 */
export type AggregateRef =
  | { type: "auto"; value: string }
  | { type: "custom"; value: string };

/** tier → 聚合选择（C2）。未设置的 tier 缺省。 */
export interface TierSelection {
  sonnet?: AggregateRef;
  opus?: AggregateRef;
  haiku?: AggregateRef;
  fable?: AggregateRef;
  default?: AggregateRef;
}

/** CC 聚合模式配置（C2，`get_cc_aggregate_config` 返回 / `set_cc_aggregate_config` 入参）。 */
export interface CcAggregateConfig {
  /** 聚合模式总开关（默认关闭）。 */
  enabled: boolean;
  tierSelection: TierSelection;
}

/** tier 名称集合（与 TierSelection 键对齐）。 */
export type TierName = "sonnet" | "opus" | "haiku" | "fable" | "default";

// ============================================================================
// API wrapper
//
// 注意：design.md 里的 `setAggregationEnabled` / `setTierSelection` 命令不存在。
// 聚合模式开关与 tier 选择都通过 `set_cc_aggregate_config` 读-改-写整个配置实现，
// 见下方 `setAggregationEnabled` / `setTierSelection` 组合方法。
// ============================================================================

export const aggregationApi = {
  // ---- C1：模型缓存 ----

  /** 列出模型缓存。providerId 为空 = 该应用全部上游的缓存行。 */
  async listProviderModels(
    appType: AppId,
    providerId?: string,
  ): Promise<ProviderModel[]> {
    return invoke("list_provider_models", {
      appType,
      providerId: providerId ?? null,
    });
  },

  /** 为某上游手动补录一条模型 id（source=manual）。 */
  async addManualModel(
    appType: AppId,
    providerId: string,
    modelId: string,
  ): Promise<void> {
    return invoke("add_manual_model", { appType, providerId, modelId });
  },

  /** 删除一条手动补录的模型（仅删 source=manual 的指定行）。 */
  async removeManualModel(
    appType: AppId,
    providerId: string,
    modelId: string,
  ): Promise<void> {
    return invoke("remove_manual_model", { appType, providerId, modelId });
  },

  /** 立即刷新：providerId 空 = 全队列，否则只刷新该上游。 */
  async refreshProviderModelsNow(
    appType: AppId,
    providerId?: string,
  ): Promise<RefreshSummary> {
    return invoke("refresh_provider_models_now", {
      appType,
      providerId: providerId ?? null,
    });
  },

  /** 读取模型缓存状态（每日全量 last-run + 各上游最近刷新时间）。 */
  async getModelCacheStatus(appType: AppId): Promise<ModelCacheStatus> {
    return invoke("get_model_cache_status", { appType });
  },

  // ---- C2：聚合派生 ----

  /** 返回全部自动聚合视图（含每个聚合的有序上游候选）。 */
  async getAggregates(appType: AppId): Promise<AggregateView[]> {
    return invoke("get_aggregates", { appType });
  },

  /** 返回全部自定义聚合派生视图（含归零标记与 missingMembers）。 */
  async getCustomAggregates(appType: AppId): Promise<CustomAggregateView[]> {
    return invoke("get_custom_aggregates", { appType });
  },

  /** 新建一个自定义聚合，返回新 id。 */
  async createCustomAggregate(
    appType: AppId,
    name: string,
    members: string[],
  ): Promise<string> {
    return invoke("create_custom_aggregate", { appType, name, members });
  },

  /**
   * 更新自定义聚合的名称和/或成员（改名/改成员/排序）。
   * 后端命令**无 appType**；name/members 均可选，undefined 传 null = 不改动该字段。
   */
  async updateCustomAggregate(
    id: string,
    name?: string,
    members?: string[],
  ): Promise<void> {
    return invoke("update_custom_aggregate", {
      id,
      name: name ?? null,
      members: members ?? null,
    });
  },

  /** 删除一个自定义聚合（后端命令**无 appType**）。 */
  async deleteCustomAggregate(id: string): Promise<void> {
    return invoke("delete_custom_aggregate", { id });
  },

  /** 按给定顺序重排自定义聚合（拖拽排序）。 */
  async reorderCustomAggregates(
    appType: AppId,
    orderedIds: string[],
  ): Promise<void> {
    return invoke("reorder_custom_aggregates", { appType, orderedIds });
  },

  /** 读取 CC 聚合模式配置（开关 + tierSelection）。 */
  async getCcAggregateConfig(appType: AppId): Promise<CcAggregateConfig> {
    return invoke("get_cc_aggregate_config", { appType });
  },

  /** 写入 CC 聚合模式配置（开关 + tierSelection）。 */
  async setCcAggregateConfig(
    appType: AppId,
    config: CcAggregateConfig,
  ): Promise<void> {
    return invoke("set_cc_aggregate_config", { appType, config });
  },

  // ---- 组合方法：读-改-写整个 CcAggregateConfig ----
  //
  // 后端没有独立的 setAggregationEnabled / setTierSelection 命令；这里先读当前
  // 配置，改动单个字段后整体写回，避免覆盖其他字段。

  /** 切换聚合模式总开关（读-改-写）。 */
  async setAggregationEnabled(appType: AppId, enabled: boolean): Promise<void> {
    const config = await this.getCcAggregateConfig(appType);
    await this.setCcAggregateConfig(appType, { ...config, enabled });
  },

  /**
   * 设置某个 tier 的聚合指向（读-改-写）。
   * ref 为 null/undefined 时清除该 tier 的设置（回到「未设置」）。
   */
  async setTierSelection(
    appType: AppId,
    tier: TierName,
    ref: AggregateRef | null,
  ): Promise<void> {
    const config = await this.getCcAggregateConfig(appType);
    const tierSelection: TierSelection = { ...config.tierSelection };
    if (ref) {
      tierSelection[tier] = ref;
    } else {
      delete tierSelection[tier];
    }
    await this.setCcAggregateConfig(appType, { ...config, tierSelection });
  },
};
