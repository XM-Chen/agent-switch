/**
 * CC 聚合模型页查询状态（C4）
 *
 * 聚合是派生物：任何源 mutation（加/删手动模型、刷新、改自定义聚合）都要交叉失效
 * `aggregates` + `customAggregates` + `providerModels` + `modelCacheStatus`。
 * 聚合模式开关照 `failover.ts` 的乐观更新 + 回滚模板写。
 */

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import {
  aggregationApi,
  type AggregateRef,
  type CcAggregateConfig,
  type TierName,
} from "@/lib/api/aggregation";
import type { AppId } from "@/lib/api";
import { extractErrorMessage } from "@/utils/errorUtils";

// ========== query keys ==========

export const aggregationKeys = {
  aggregates: (appType: AppId) => ["aggregates", appType] as const,
  customAggregates: (appType: AppId) => ["customAggregates", appType] as const,
  ccAggregateConfig: (appType: AppId) =>
    ["ccAggregateConfig", appType] as const,
  modelCacheStatus: (appType: AppId) => ["modelCacheStatus", appType] as const,
  providerModels: (appType: AppId, providerId?: string) =>
    ["providerModels", appType, providerId ?? null] as const,
};

/**
 * 聚合是派生物：任何源变更都要重取聚合 + 自定义聚合 + 模型缓存。
 * 集中一处，避免各 mutation 漏失效。
 *
 * 导出供聚合页外的「源」mutation 复用：故障转移队列增删/排序、删除 provider
 * 等也会改变聚合派生结果，需在其 `onSuccess`/`onSettled` 交叉失效聚合 query。
 */
export function invalidateAggregationSources(
  queryClient: ReturnType<typeof useQueryClient>,
  appType: AppId,
) {
  queryClient.invalidateQueries({
    queryKey: aggregationKeys.aggregates(appType),
  });
  queryClient.invalidateQueries({
    queryKey: aggregationKeys.customAggregates(appType),
  });
  queryClient.invalidateQueries({
    queryKey: aggregationKeys.modelCacheStatus(appType),
  });
  // providerModels 有 providerId 维度，失效整个前缀。
  queryClient.invalidateQueries({ queryKey: ["providerModels", appType] });
}

// ========== queries ==========

/**
 * 自动聚合列表。聚合是派生物，代理运行时轮询 5s 保持与后端一致。
 */
export function useAggregatesQuery(appType: AppId, isProxyRunning: boolean) {
  return useQuery({
    queryKey: aggregationKeys.aggregates(appType),
    queryFn: () => aggregationApi.getAggregates(appType),
    enabled: !!appType,
    refetchInterval: isProxyRunning ? 5000 : false,
  });
}

/** 自定义聚合派生视图列表。 */
export function useCustomAggregatesQuery(appType: AppId) {
  return useQuery({
    queryKey: aggregationKeys.customAggregates(appType),
    queryFn: () => aggregationApi.getCustomAggregates(appType),
    enabled: !!appType,
  });
}

/** CC 聚合模式配置（开关 + tierSelection）。 */
export function useCcAggregateConfigQuery(appType: AppId) {
  return useQuery({
    queryKey: aggregationKeys.ccAggregateConfig(appType),
    queryFn: () => aggregationApi.getCcAggregateConfig(appType),
    enabled: !!appType,
  });
}

/** 模型缓存状态（每日全量 last-run + 各上游最近刷新时间）。 */
export function useModelCacheStatusQuery(appType: AppId) {
  return useQuery({
    queryKey: aggregationKeys.modelCacheStatus(appType),
    queryFn: () => aggregationApi.getModelCacheStatus(appType),
    enabled: !!appType,
  });
}

/** 某上游的模型缓存行（用于手动加模型子界面展示 manual/fetched 行）。 */
export function useProviderModelsQuery(
  appType: AppId,
  providerId?: string,
  enabled = true,
) {
  return useQuery({
    queryKey: aggregationKeys.providerModels(appType, providerId),
    queryFn: () => aggregationApi.listProviderModels(appType, providerId),
    enabled: !!appType && enabled,
  });
}

// ========== 手动模型 mutations ==========

/** 手动补录模型。成功后交叉失效聚合派生 + 该上游模型缓存。 */
export function useAddManualModel() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: ({
      appType,
      providerId,
      modelId,
    }: {
      appType: AppId;
      providerId: string;
      modelId: string;
    }) => aggregationApi.addManualModel(appType, providerId, modelId),
    onSuccess: () => {
      toast.success(
        t("aggregation.manualModel.added", {
          defaultValue: "已添加手动模型",
        }),
        { closeButton: true },
      );
    },
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("aggregation.manualModel.addFailed", {
          detail,
          defaultValue: `添加手动模型失败: ${detail}`,
        }),
      );
    },
    onSettled: (_d, _e, variables) => {
      invalidateAggregationSources(queryClient, variables.appType);
    },
  });
}

/** 删除手动模型。 */
export function useRemoveManualModel() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: ({
      appType,
      providerId,
      modelId,
    }: {
      appType: AppId;
      providerId: string;
      modelId: string;
    }) => aggregationApi.removeManualModel(appType, providerId, modelId),
    onSuccess: () => {
      toast.success(
        t("aggregation.manualModel.removed", {
          defaultValue: "已删除手动模型",
        }),
        { closeButton: true },
      );
    },
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("aggregation.manualModel.removeFailed", {
          detail,
          defaultValue: `删除手动模型失败: ${detail}`,
        }),
      );
    },
    onSettled: (_d, _e, variables) => {
      invalidateAggregationSources(queryClient, variables.appType);
    },
  });
}

// ========== 刷新 mutation ==========

/** 手动刷新（providerId 空 = 全队列，否则单上游）。 */
export function useRefreshModels() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: ({
      appType,
      providerId,
    }: {
      appType: AppId;
      providerId?: string;
    }) => aggregationApi.refreshProviderModelsNow(appType, providerId),
    onSuccess: (summary) => {
      toast.success(
        t("aggregation.refresh.done", {
          refreshed: summary.refreshed,
          skipped: summary.skipped,
          total: summary.totalModels,
          defaultValue: `刷新完成：${summary.refreshed} 个上游成功，${summary.skipped} 个跳过，共 ${summary.totalModels} 个模型`,
        }),
        { closeButton: true },
      );
    },
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("aggregation.refresh.failed", {
          detail,
          defaultValue: `刷新失败: ${detail}`,
        }),
      );
    },
    onSettled: (_d, _e, variables) => {
      invalidateAggregationSources(queryClient, variables.appType);
    },
  });
}

// ========== 自定义聚合 mutations ==========

/** 新建自定义聚合。 */
export function useCreateCustomAggregate() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: ({
      appType,
      name,
      members,
    }: {
      appType: AppId;
      name: string;
      members: string[];
    }) => aggregationApi.createCustomAggregate(appType, name, members),
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("aggregation.custom.createFailed", {
          detail,
          defaultValue: `创建自定义聚合失败: ${detail}`,
        }),
      );
    },
    onSettled: (_d, _e, variables) => {
      invalidateAggregationSources(queryClient, variables.appType);
    },
  });
}

/** 更新自定义聚合（改名/改成员/排序）。后端命令无 appType，失效需显式带 appType。 */
export function useUpdateCustomAggregate() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: ({
      id,
      name,
      members,
    }: {
      appType: AppId;
      id: string;
      name?: string;
      members?: string[];
    }) => aggregationApi.updateCustomAggregate(id, name, members),
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("aggregation.custom.updateFailed", {
          detail,
          defaultValue: `更新自定义聚合失败: ${detail}`,
        }),
      );
    },
    onSettled: (_d, _e, variables) => {
      invalidateAggregationSources(queryClient, variables.appType);
    },
  });
}

/** 删除自定义聚合（用户显式删除）。后端命令无 appType。 */
export function useDeleteCustomAggregate() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: ({ id }: { appType: AppId; id: string }) =>
      aggregationApi.deleteCustomAggregate(id),
    onSuccess: () => {
      toast.success(
        t("aggregation.custom.deleted", {
          defaultValue: "已删除自定义聚合",
        }),
        { closeButton: true },
      );
    },
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("aggregation.custom.deleteFailed", {
          detail,
          defaultValue: `删除自定义聚合失败: ${detail}`,
        }),
      );
    },
    onSettled: (_d, _e, variables) => {
      invalidateAggregationSources(queryClient, variables.appType);
    },
  });
}

/** 重排自定义聚合。 */
export function useReorderCustomAggregates() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: ({
      appType,
      orderedIds,
    }: {
      appType: AppId;
      orderedIds: string[];
    }) => aggregationApi.reorderCustomAggregates(appType, orderedIds),
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("aggregation.custom.reorderFailed", {
          detail,
          defaultValue: `排序失败: ${detail}`,
        }),
      );
    },
    onSettled: (_d, _e, variables) => {
      queryClient.invalidateQueries({
        queryKey: aggregationKeys.customAggregates(variables.appType),
      });
    },
  });
}

// ========== tier 选择 mutation（读-改-写整个 CcAggregateConfig） ==========

/** 设置某个 tier 的聚合指向（ref 为 null 清除）。 */
export function useSetTierSelection() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: ({
      appType,
      tier,
      ref,
    }: {
      appType: AppId;
      tier: TierName;
      ref: AggregateRef | null;
    }) => aggregationApi.setTierSelection(appType, tier, ref),
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("aggregation.tier.setFailed", {
          detail,
          defaultValue: `设置档位失败: ${detail}`,
        }),
      );
    },
    onSettled: (_d, _e, variables) => {
      queryClient.invalidateQueries({
        queryKey: aggregationKeys.ccAggregateConfig(variables.appType),
      });
    },
  });
}

// ========== 聚合模式开关（照 failover.ts 乐观更新 + 回滚模板） ==========

/**
 * 切换聚合模式总开关。乐观更新 ccAggregateConfig 的 enabled 字段，失败回滚。
 * 底层是 `set_cc_aggregate_config` 读-改-写，避免覆盖 tierSelection。
 */
export function useSetAggregationEnabled() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  return useMutation({
    mutationFn: ({ appType, enabled }: { appType: AppId; enabled: boolean }) =>
      aggregationApi.setAggregationEnabled(appType, enabled),

    // 乐观更新
    onMutate: async ({ appType, enabled }) => {
      const key = aggregationKeys.ccAggregateConfig(appType);
      await queryClient.cancelQueries({ queryKey: key });
      const previousValue = queryClient.getQueryData<CcAggregateConfig>(key);
      if (previousValue) {
        queryClient.setQueryData<CcAggregateConfig>(key, {
          ...previousValue,
          enabled,
        });
      }
      return { previousValue, appType };
    },

    onSuccess: (_data, variables) => {
      toast.success(
        variables.enabled
          ? t("aggregation.mode.enabled", {
              defaultValue: "聚合模式已启用",
            })
          : t("aggregation.mode.disabled", {
              defaultValue: "聚合模式已关闭",
            }),
        { closeButton: true },
      );
    },

    // 错误时回滚
    onError: (error: Error, _variables, context) => {
      if (context?.previousValue !== undefined && context.previousValue) {
        queryClient.setQueryData(
          aggregationKeys.ccAggregateConfig(context.appType),
          context.previousValue,
        );
      }
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("aggregation.mode.toggleFailed", {
          detail,
          defaultValue: `操作失败: ${detail}`,
        }),
      );
    },

    // 无论成败都重新获取
    onSettled: (_d, _e, variables) => {
      queryClient.invalidateQueries({
        queryKey: aggregationKeys.ccAggregateConfig(variables.appType),
      });
    },
  });
}
