/**
 * CC 聚合模型页（C4）
 *
 * 三段式：聚合总览（开关 + tier 选择 + 全局刷新）、自动聚合列表、自定义聚合。
 * 消费 C1/C2/C3 已注册的 Tauri 命令，不改后端。仅 claude。
 */

import { useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import {
  Boxes,
  Loader2,
  Plus,
  Pencil,
  Trash2,
  RefreshCw,
  Sparkles,
  AlertTriangle,
  ChevronDown,
} from "lucide-react";
import { Button } from "@/components/ui/button";
import { Badge } from "@/components/ui/badge";
import { Card, CardContent } from "@/components/ui/card";
import { ToggleRow } from "@/components/ui/toggle-row";
import { Alert, AlertDescription } from "@/components/ui/alert";
import { Tabs, TabsContent, TabsList, TabsTrigger } from "@/components/ui/tabs";
import {
  Select,
  SelectContent,
  SelectGroup,
  SelectItem,
  SelectLabel,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { ProviderIcon } from "@/components/ProviderIcon";
import { ConfirmDialog } from "@/components/ConfirmDialog";
import type { AppId } from "@/lib/api";
import type {
  AggregateView,
  CustomAggregateView,
  TierName,
} from "@/lib/api/aggregation";
import { useProxyStatus } from "@/hooks/useProxyStatus";
import { getProxyTakeoverState } from "@/types/proxy";
import {
  useAggregatesQuery,
  useCustomAggregatesQuery,
  useCcAggregateConfigQuery,
  useModelCacheStatusQuery,
  useSetAggregationEnabled,
  useSetTierSelection,
  useRefreshModels,
  useDeleteCustomAggregate,
} from "@/lib/query/aggregation";
import {
  buildTierOptions,
  decodeAggregateRef,
  resolveTierRef,
  tierRefToSelectValue,
  formatFetchedAt,
  formatRfc3339,
  TIER_NONE_VALUE,
} from "./aggregationHelpers";
import { ManualModelManager } from "./ManualModelManager";
import { CustomAggregateDialog } from "./CustomAggregateDialog";

const TIER_NAMES: TierName[] = ["opus", "sonnet", "haiku", "fable", "default"];

interface AggregationPanelProps {
  appId: AppId;
}

export function AggregationPanel({ appId }: AggregationPanelProps) {
  const { t } = useTranslation();
  const { isRunning: isProxyRunning, takeoverStatus } = useProxyStatus();
  const takeoverEnabled =
    getProxyTakeoverState(takeoverStatus, appId)?.takeoverEnabled ?? false;

  const { data: aggregates = [], isLoading: aggregatesLoading } =
    useAggregatesQuery(appId, isProxyRunning);
  const { data: customAggregates = [] } = useCustomAggregatesQuery(appId);
  const { data: config } = useCcAggregateConfigQuery(appId);
  const { data: cacheStatus } = useModelCacheStatusQuery(appId);

  const setEnabled = useSetAggregationEnabled();
  const setTier = useSetTierSelection();
  const refreshModels = useRefreshModels();
  const deleteCustom = useDeleteCustomAggregate();

  const [dialogOpen, setDialogOpen] = useState(false);
  const [editing, setEditing] = useState<CustomAggregateView | null>(null);
  const [pendingDelete, setPendingDelete] =
    useState<CustomAggregateView | null>(null);

  const enabled = config?.enabled ?? false;
  const tierOptions = useMemo(
    () => buildTierOptions(aggregates, customAggregates),
    [aggregates, customAggregates],
  );

  const handleToggleMode = (checked: boolean) => {
    if (checked && !takeoverEnabled) return;
    setEnabled.mutate({ appType: appId, enabled: checked });
  };

  const handleTierChange = (tier: TierName, value: string) => {
    setTier.mutate({ appType: appId, tier, ref: decodeAggregateRef(value) });
  };

  const handleGlobalRefresh = () => {
    refreshModels.mutate({ appType: appId });
  };

  const openCreate = () => {
    setEditing(null);
    setDialogOpen(true);
  };

  const openEdit = (view: CustomAggregateView) => {
    setEditing(view);
    setDialogOpen(true);
  };

  // C2 的 CustomAggregateView 直接回传原始 orderedMembers（含已归零成员，保序），
  // 编辑对话框据此精确还原成员顺序，无需前端近似重建。
  const editingMemberKeys = useMemo(
    () => editing?.orderedMembers ?? [],
    [editing],
  );

  return (
    <div className="flex flex-1 flex-col gap-4 px-6 pt-4 pb-12">
      <Tabs defaultValue="overview" className="flex flex-1 flex-col">
        <TabsList className="self-start">
          <TabsTrigger value="overview">
            {t("aggregation.tabs.overview", { defaultValue: "聚合总览" })}
          </TabsTrigger>
          <TabsTrigger value="auto">
            {t("aggregation.tabs.auto", { defaultValue: "自动聚合" })}
          </TabsTrigger>
          <TabsTrigger value="custom">
            {t("aggregation.tabs.custom", { defaultValue: "自定义聚合" })}
          </TabsTrigger>
        </TabsList>

        {/* 段一：聚合总览 */}
        <TabsContent value="overview" className="space-y-4">
          <ToggleRow
            icon={
              <Sparkles
                className={
                  enabled
                    ? "h-4 w-4 text-emerald-500"
                    : "h-4 w-4 text-muted-foreground"
                }
              />
            }
            title={t("aggregation.mode.title", { defaultValue: "聚合模式" })}
            description={
              !takeoverEnabled
                ? t("aggregation.mode.takeoverRequired", {
                    defaultValue:
                      "请先接管 Claude Code 并启动路由服务，再启用聚合模式。",
                  })
                : enabled
                  ? t("aggregation.mode.enabledDesc", {
                      defaultValue:
                        "已按 tier→聚合 配置路由；请求会在聚合内候选间按优先级故障转移。",
                    })
                  : t("aggregation.mode.disabledDesc", {
                      defaultValue:
                        "关闭后退化为普通行为（切当前供应商 / 直连），不走聚合路由。",
                    })
            }
            checked={enabled}
            onCheckedChange={handleToggleMode}
            disabled={setEnabled.isPending || !takeoverEnabled}
          />

          {!takeoverEnabled && (
            <Alert className="border-amber-500/40 bg-amber-500/10">
              <AlertTriangle className="h-4 w-4 text-amber-500" />
              <AlertDescription className="text-amber-800 dark:text-amber-200">
                {t("aggregation.mode.proxyHint", {
                  defaultValue:
                    "聚合路由依赖本地路由服务：请在首页开启「路由服务」并接管 Claude Code 后再启用聚合模式。",
                })}
              </AlertDescription>
            </Alert>
          )}

          {/* tier→聚合 选择 */}
          <Card>
            <CardContent className="space-y-3 p-4">
              <div className="flex items-center justify-between">
                <h3 className="text-sm font-medium">
                  {t("aggregation.tier.title", {
                    defaultValue: "档位 → 聚合",
                  })}
                </h3>
                <span className="text-xs text-muted-foreground">
                  {t("aggregation.tier.hint", {
                    defaultValue: "为每个模型档位指定一个聚合",
                  })}
                </span>
              </div>
              <div className="grid gap-3 sm:grid-cols-2">
                {TIER_NAMES.map((tier) => (
                  <TierSelector
                    key={tier}
                    tier={tier}
                    value={tierRefToSelectValue(config?.tierSelection?.[tier])}
                    options={tierOptions}
                    resolved={resolveTierRef(
                      config?.tierSelection?.[tier],
                      aggregates,
                      customAggregates,
                    )}
                    disabled={setTier.isPending}
                    onChange={(value) => handleTierChange(tier, value)}
                  />
                ))}
              </div>
            </CardContent>
          </Card>

          {/* 全局刷新 + last-run */}
          <Card>
            <CardContent className="flex flex-wrap items-center justify-between gap-3 p-4">
              <div className="space-y-1">
                <h3 className="text-sm font-medium">
                  {t("aggregation.refresh.title", {
                    defaultValue: "模型缓存刷新",
                  })}
                </h3>
                <p className="text-xs text-muted-foreground">
                  {cacheStatus?.lastFullRefresh
                    ? t("aggregation.refresh.lastRun", {
                        time: formatRfc3339(cacheStatus.lastFullRefresh),
                        defaultValue: `上次全量刷新：${formatRfc3339(cacheStatus.lastFullRefresh)}`,
                      })
                    : t("aggregation.refresh.neverRun", {
                        defaultValue: "尚未执行过全量刷新",
                      })}
                </p>
              </div>
              <Button
                variant="outline"
                size="sm"
                onClick={handleGlobalRefresh}
                disabled={refreshModels.isPending}
              >
                {refreshModels.isPending ? (
                  <Loader2 className="h-4 w-4 animate-spin" />
                ) : (
                  <RefreshCw className="h-4 w-4" />
                )}
                {t("aggregation.refresh.button", {
                  defaultValue: "刷新全部队列上游",
                })}
              </Button>
            </CardContent>
          </Card>
        </TabsContent>

        {/* 段二：自动聚合列表 */}
        <TabsContent value="auto" className="space-y-3">
          {aggregatesLoading ? (
            <div className="flex items-center gap-2 text-sm text-muted-foreground">
              <Loader2 className="h-4 w-4 animate-spin" />
              {t("common.loading", { defaultValue: "加载中…" })}
            </div>
          ) : aggregates.length === 0 ? (
            <EmptyHint
              text={t("aggregation.auto.empty", {
                defaultValue:
                  "暂无自动聚合。请先在首页把供应商加入故障转移队列，并确保已抓取到模型。",
              })}
            />
          ) : (
            aggregates.map((agg) => (
              <AutoAggregateCard
                key={agg.key}
                appId={appId}
                aggregate={agg}
                cacheStatus={cacheStatus}
              />
            ))
          )}
        </TabsContent>

        {/* 段三：自定义聚合 */}
        <TabsContent value="custom" className="space-y-3">
          <div className="flex items-center justify-between">
            <p className="text-xs text-muted-foreground">
              {t("aggregation.custom.sectionHint", {
                defaultValue:
                  "自定义聚合由多个自动聚合按顺序组成，路由按成员顺序依次尝试。",
              })}
            </p>
            <Button
              size="sm"
              onClick={openCreate}
              disabled={aggregates.length === 0}
            >
              <Plus className="h-4 w-4" />
              {t("aggregation.custom.create", { defaultValue: "新建" })}
            </Button>
          </div>

          {customAggregates.length === 0 ? (
            <EmptyHint
              text={t("aggregation.custom.empty", {
                defaultValue: "暂无自定义聚合。",
              })}
            />
          ) : (
            customAggregates.map((view) => (
              <CustomAggregateCard
                key={view.id}
                view={view}
                onEdit={() => openEdit(view)}
                onDelete={() => setPendingDelete(view)}
              />
            ))
          )}
        </TabsContent>
      </Tabs>

      <CustomAggregateDialog
        open={dialogOpen}
        onOpenChange={setDialogOpen}
        appId={appId}
        aggregates={aggregates}
        editing={editing}
        editingMemberKeys={editingMemberKeys}
      />

      <ConfirmDialog
        isOpen={Boolean(pendingDelete)}
        title={t("aggregation.custom.deleteTitle", {
          defaultValue: "删除自定义聚合",
        })}
        message={t("aggregation.custom.deleteMessage", {
          name: pendingDelete?.name ?? "",
          defaultValue: `确定删除自定义聚合「${pendingDelete?.name ?? ""}」吗？此操作不可撤销。`,
        })}
        onConfirm={() => {
          if (pendingDelete) {
            deleteCustom.mutate({ appType: appId, id: pendingDelete.id });
          }
          setPendingDelete(null);
        }}
        onCancel={() => setPendingDelete(null)}
      />
    </div>
  );
}

function EmptyHint({ text }: { text: string }) {
  return (
    <div className="rounded-lg border border-dashed border-border px-6 py-8 text-center text-sm text-muted-foreground">
      {text}
    </div>
  );
}

interface TierSelectorProps {
  tier: TierName;
  value: string;
  options: ReturnType<typeof buildTierOptions>;
  resolved: ReturnType<typeof resolveTierRef>;
  disabled: boolean;
  onChange: (value: string) => void;
}

function TierSelector({
  tier,
  value,
  options,
  resolved,
  disabled,
  onChange,
}: TierSelectorProps) {
  const { t } = useTranslation();
  const tierLabel = t(`aggregation.tier.names.${tier}`, {
    defaultValue: tier,
  });
  const isUnset = value === TIER_NONE_VALUE;
  const isStale = resolved !== null && !resolved.resolved;

  return (
    <div className="space-y-1.5">
      <div className="flex items-center gap-2">
        <span className="text-sm font-medium">{tierLabel}</span>
        {isUnset && (
          <Badge variant="outline" className="text-[10px]">
            {t("aggregation.tier.unset", { defaultValue: "未设置" })}
          </Badge>
        )}
        {resolved?.isEmpty && !isUnset && (
          <Badge variant="destructive" className="text-[10px]">
            {t("aggregation.tier.emptyAggregate", { defaultValue: "空聚合" })}
          </Badge>
        )}
      </div>
      <Select value={value} onValueChange={onChange} disabled={disabled}>
        <SelectTrigger>
          <SelectValue />
        </SelectTrigger>
        <SelectContent>
          <SelectItem value={TIER_NONE_VALUE}>
            {t("aggregation.tier.unsetOption", { defaultValue: "（未设置）" })}
          </SelectItem>
          {isStale && (
            <SelectItem value={value}>
              {t("aggregation.tier.staleOption", {
                label: resolved?.label ?? "",
                defaultValue: `${resolved?.label ?? ""}（已失效）`,
              })}
            </SelectItem>
          )}
          {options.auto.length > 0 && (
            <SelectGroup>
              <SelectLabel>
                {t("aggregation.tier.autoGroup", { defaultValue: "自动聚合" })}
              </SelectLabel>
              {options.auto.map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>
                  {opt.label}
                  {opt.isEmpty
                    ? t("aggregation.tier.emptySuffix", {
                        defaultValue: "（空）",
                      })
                    : ""}
                </SelectItem>
              ))}
            </SelectGroup>
          )}
          {options.custom.length > 0 && (
            <SelectGroup>
              <SelectLabel>
                {t("aggregation.tier.customGroup", {
                  defaultValue: "自定义聚合",
                })}
              </SelectLabel>
              {options.custom.map((opt) => (
                <SelectItem key={opt.value} value={opt.value}>
                  {opt.label}
                  {opt.isEmpty
                    ? t("aggregation.tier.emptySuffix", {
                        defaultValue: "（空）",
                      })
                    : ""}
                </SelectItem>
              ))}
            </SelectGroup>
          )}
        </SelectContent>
      </Select>
      {isUnset && (
        <p className="text-[11px] text-muted-foreground">
          {t("aggregation.tier.unsetConsequence", {
            defaultValue:
              "未设置：聚合模式下该档位请求不改写，按普通路由处理。",
          })}
        </p>
      )}
    </div>
  );
}

interface AutoAggregateCardProps {
  appId: AppId;
  aggregate: AggregateView;
  cacheStatus?: import("@/lib/api/aggregation").ModelCacheStatus;
}

function AutoAggregateCard({
  appId,
  aggregate,
  cacheStatus,
}: AutoAggregateCardProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);
  const [manualFor, setManualFor] = useState<string | null>(null);

  const latestByProvider = useMemo(() => {
    const map = new Map<string, number>();
    for (const p of cacheStatus?.providers ?? []) {
      map.set(p.providerId, p.latestFetchedAt);
    }
    return map;
  }, [cacheStatus]);

  // 聚合内出现的去重上游（手动加模型入口按上游展开）。
  const uniqueProviders = useMemo(() => {
    const seen = new Map<string, string>();
    for (const m of aggregate.members) {
      if (!seen.has(m.providerId)) seen.set(m.providerId, m.providerName);
    }
    return Array.from(seen.entries()).map(([providerId, providerName]) => ({
      providerId,
      providerName,
    }));
  }, [aggregate.members]);

  return (
    <Card>
      <CardContent className="p-0">
        <button
          type="button"
          className="flex w-full items-center justify-between gap-2 px-4 py-3 text-left"
          onClick={() => setExpanded((v) => !v)}
        >
          <div className="flex min-w-0 items-center gap-2">
            <Boxes className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="truncate font-mono text-sm">{aggregate.key}</span>
            <Badge variant="secondary" className="shrink-0">
              {t("aggregation.auto.candidateCount", {
                count: aggregate.members.length,
                defaultValue: `${aggregate.members.length} 个候选`,
              })}
            </Badge>
          </div>
          <ChevronDown
            className={`h-4 w-4 shrink-0 text-muted-foreground transition-transform ${
              expanded ? "rotate-180" : ""
            }`}
          />
        </button>

        {expanded && (
          <div className="space-y-3 border-t border-border px-4 py-3">
            <ul className="space-y-1.5">
              {aggregate.members.map((member, index) => {
                const latest = latestByProvider.get(member.providerId);
                return (
                  <li
                    key={`${member.providerId}:${member.modelId}`}
                    className="flex items-center justify-between gap-2 text-sm"
                  >
                    <div className="flex min-w-0 items-center gap-2">
                      <Badge variant="outline" className="shrink-0">
                        P{index + 1}
                      </Badge>
                      <ProviderIcon
                        name={member.providerName}
                        size={16}
                        className="shrink-0"
                      />
                      <span className="shrink-0 text-muted-foreground">
                        {member.providerName}
                      </span>
                      <span className="text-muted-foreground">→</span>
                      <span className="min-w-0 truncate font-mono text-xs">
                        {member.modelId}
                      </span>
                      <Badge
                        variant={
                          member.source === "manual" ? "secondary" : "outline"
                        }
                        className="shrink-0 text-[10px]"
                      >
                        {member.source === "manual"
                          ? t("aggregation.manualModel.manualTag", {
                              defaultValue: "手动",
                            })
                          : t("aggregation.manualModel.fetchedTag", {
                              defaultValue: "自动",
                            })}
                      </Badge>
                    </div>
                    {latest && latest > 0 && (
                      <span className="shrink-0 text-[11px] text-muted-foreground">
                        {formatFetchedAt(latest)}
                      </span>
                    )}
                  </li>
                );
              })}
            </ul>

            {/* 手动加/删模型：按聚合内出现的各上游分别提供入口 */}
            <div className="space-y-2 border-t border-dashed border-border pt-3">
              <div className="flex flex-wrap items-center gap-1.5">
                <span className="text-xs text-muted-foreground">
                  {t("aggregation.manualModel.manageFor", {
                    defaultValue: "手动补录模型到：",
                  })}
                </span>
                {uniqueProviders.map((p) => (
                  <Button
                    key={p.providerId}
                    variant={
                      manualFor === p.providerId ? "secondary" : "outline"
                    }
                    size="sm"
                    className="h-7"
                    onClick={() =>
                      setManualFor((cur) =>
                        cur === p.providerId ? null : p.providerId,
                      )
                    }
                  >
                    {p.providerName}
                  </Button>
                ))}
              </div>
              {manualFor && (
                <ManualModelManager appId={appId} providerId={manualFor} />
              )}
            </div>
          </div>
        )}
      </CardContent>
    </Card>
  );
}

interface CustomAggregateCardProps {
  view: CustomAggregateView;
  onEdit: () => void;
  onDelete: () => void;
}

function CustomAggregateCard({
  view,
  onEdit,
  onDelete,
}: CustomAggregateCardProps) {
  const { t } = useTranslation();
  const [expanded, setExpanded] = useState(false);

  return (
    <Card>
      <CardContent className="p-0">
        <div className="flex items-center justify-between gap-2 px-4 py-3">
          <button
            type="button"
            className="flex min-w-0 flex-1 items-center gap-2 text-left"
            onClick={() => setExpanded((v) => !v)}
          >
            <Boxes className="h-4 w-4 shrink-0 text-muted-foreground" />
            <span className="truncate text-sm font-medium">{view.name}</span>
            <Badge variant="secondary" className="shrink-0">
              {t("aggregation.auto.candidateCount", {
                count: view.members.length,
                defaultValue: `${view.members.length} 个候选`,
              })}
            </Badge>
            {view.isEmpty && (
              <Badge variant="destructive" className="shrink-0 text-[10px]">
                {t("aggregation.custom.emptyTag", {
                  defaultValue: "成员已全部下线",
                })}
              </Badge>
            )}
            <ChevronDown
              className={`h-4 w-4 shrink-0 text-muted-foreground transition-transform ${
                expanded ? "rotate-180" : ""
              }`}
            />
          </button>
          <div className="flex shrink-0 items-center gap-1">
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7"
              onClick={onEdit}
              title={t("common.edit", { defaultValue: "编辑" })}
            >
              <Pencil className="h-3.5 w-3.5" />
            </Button>
            <Button
              variant="ghost"
              size="icon"
              className="h-7 w-7 text-muted-foreground hover:text-destructive"
              onClick={onDelete}
              title={t("common.delete", { defaultValue: "删除" })}
            >
              <Trash2 className="h-3.5 w-3.5" />
            </Button>
          </div>
        </div>

        {view.isEmpty && (
          <div className="border-t border-border px-4 py-3">
            <Alert className="border-amber-500/40 bg-amber-500/10">
              <AlertTriangle className="h-4 w-4 text-amber-500" />
              <AlertDescription className="text-amber-800 dark:text-amber-200">
                {t("aggregation.custom.emptyAlert", {
                  defaultValue:
                    "该聚合当前所有成员均已归零（不再有候选），但定义保留；恢复成员后自动生效。",
                })}
              </AlertDescription>
            </Alert>
          </div>
        )}

        {view.missingMembers.length > 0 && (
          <div className="border-t border-border px-4 pt-3">
            <p className="text-[11px] text-amber-600 dark:text-amber-400">
              {t("aggregation.custom.missingMembers", {
                members: view.missingMembers.join("、"),
                defaultValue: `已归零成员：${view.missingMembers.join("、")}`,
              })}
            </p>
          </div>
        )}

        {expanded && view.members.length > 0 && (
          <ul className="space-y-1.5 border-t border-border px-4 py-3">
            {view.members.map((member, index) => (
              <li
                key={`${member.providerId}:${member.modelId}`}
                className="flex items-center gap-2 text-sm"
              >
                <Badge variant="outline" className="shrink-0">
                  P{index + 1}
                </Badge>
                <ProviderIcon
                  name={member.providerName}
                  size={16}
                  className="shrink-0"
                />
                <span className="shrink-0 text-muted-foreground">
                  {member.providerName}
                </span>
                <span className="text-muted-foreground">→</span>
                <span className="min-w-0 truncate font-mono text-xs">
                  {member.modelId}
                </span>
              </li>
            ))}
          </ul>
        )}
      </CardContent>
    </Card>
  );
}
