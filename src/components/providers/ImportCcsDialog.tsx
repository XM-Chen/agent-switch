import { useEffect, useMemo, useState } from "react";
import { useTranslation } from "react-i18next";
import { useQuery, useQueryClient } from "@tanstack/react-query";
import { toast } from "sonner";
import { AlertTriangle, Loader2 } from "lucide-react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { Checkbox } from "@/components/ui/checkbox";
import { Alert, AlertDescription } from "@/components/ui/alert";
import {
  providersApi,
  type CcsDetectItem,
  type CcsDetectResponse,
} from "@/lib/api/providers";

interface ImportCcsDialogProps {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}

/** 三态标签的展示样式。 */
function statusBadgeClass(status: CcsDetectItem["status"]): string {
  switch (status) {
    case "new":
      return "bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-300";
    case "update":
      return "bg-blue-100 dark:bg-blue-900/30 text-blue-700 dark:text-blue-300";
    default:
      return "bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-400";
  }
}

/**
 * 从本机 cc-switch 一键同步 Claude 上游渠道。
 *
 * 打开时探测 ccs 数据源，展示三态（new/update/unchanged）预览列表：
 * - 可导入项默认勾选（unchanged 除外——无变化无需再同步）；
 * - 不可导入项（缺 base_url）禁用勾选并给出原因。
 * 确认后逐项同步，结果按 created/updated/skipped/errors 反馈，成功后失效
 * `["providers", "claude"]` 查询。全程不改当前渠道、不写 live 配置。
 */
export function ImportCcsDialog({ open, onOpenChange }: ImportCcsDialogProps) {
  const { t } = useTranslation();
  const queryClient = useQueryClient();

  const [selected, setSelected] = useState<Record<string, boolean>>({});
  const [isSyncing, setIsSyncing] = useState(false);

  const { data, isLoading, error, refetch } = useQuery<CcsDetectResponse>({
    queryKey: ["ccs-detect"],
    queryFn: () => providersApi.detectCcsChannels(),
    enabled: open,
    retry: 1,
    gcTime: 0,
    staleTime: 0,
  });

  // 每次打开或探测结果变化时，重置勾选：可导入且非 unchanged 的默认勾选。
  useEffect(() => {
    if (!open) return;
    const next: Record<string, boolean> = {};
    for (const item of data?.providers ?? []) {
      next[item.originalId] = item.importable && item.status !== "unchanged";
    }
    setSelected(next);
  }, [open, data]);

  const items = useMemo(() => data?.providers ?? [], [data]);
  const selectableCount = useMemo(
    () => items.filter((i) => i.importable && i.status !== "unchanged").length,
    [items],
  );
  const selectedCount = useMemo(
    () => items.filter((i) => selected[i.originalId]).length,
    [items, selected],
  );

  const toggle = (item: CcsDetectItem) => {
    if (!item.importable) return;
    setSelected((prev) => ({
      ...prev,
      [item.originalId]: !prev[item.originalId],
    }));
  };

  const handleSync = async () => {
    const toSync = items.filter((i) => i.importable && selected[i.originalId]);
    if (toSync.length === 0) return;

    setIsSyncing(true);
    try {
      const result = await providersApi.syncCcsChannels(
        toSync.map((i) => ({
          originalId: i.originalId,
          importedName: i.importedName,
        })),
      );

      const createdN = result.created.length;
      const updatedN = result.updated.length;
      const errorsN = result.errors.length;

      if (createdN > 0 || updatedN > 0) {
        await queryClient.invalidateQueries({
          queryKey: ["providers", "claude"],
        });
      }

      if (errorsN > 0) {
        toast.warning(
          t("importCcs.partialSuccess", {
            defaultValue: "部分渠道同步失败",
          }),
          {
            description: t("importCcs.resultSummary", {
              defaultValue:
                "新增 {{created}}，更新 {{updated}}，失败 {{errors}}",
              created: createdN,
              updated: updatedN,
              errors: errorsN,
            }),
          },
        );
      } else {
        toast.success(
          t("importCcs.syncSuccess", { defaultValue: "同步完成" }),
          {
            description: t("importCcs.resultSummaryOk", {
              defaultValue: "新增 {{created}}，更新 {{updated}}",
              created: createdN,
              updated: updatedN,
            }),
            closeButton: true,
          },
        );
        onOpenChange(false);
        return;
      }

      // 有错误时保留对话框，逐项列出（借 refetch 刷新三态）。
      await refetch();
    } catch (err) {
      toast.error(t("importCcs.syncError", { defaultValue: "同步失败" }), {
        description: err instanceof Error ? err.message : String(err),
      });
    } finally {
      setIsSyncing(false);
    }
  };

  return (
    <Dialog
      open={open}
      onOpenChange={(next) => {
        if (!next && !isSyncing) onOpenChange(false);
      }}
    >
      <DialogContent zIndex="top" className="max-w-2xl">
        <DialogHeader>
          <DialogTitle>
            {t("importCcs.title", { defaultValue: "从 cc-switch 同步渠道" })}
          </DialogTitle>
          <DialogDescription>
            {t("importCcs.description", {
              defaultValue:
                "把本机 cc-switch 已保存的 Claude 渠道同步过来。不会改变当前渠道，也不会写入 Claude Code 配置。",
            })}
          </DialogDescription>
        </DialogHeader>

        <div className="max-h-[60vh] overflow-y-auto px-1 py-2">
          {isLoading ? (
            <div className="flex items-center justify-center py-10">
              <Loader2 className="h-6 w-6 animate-spin text-muted-foreground" />
            </div>
          ) : error ? (
            <Alert variant="destructive">
              <AlertDescription className="flex items-center justify-between gap-3">
                <span>
                  {t("importCcs.detectError", {
                    defaultValue: "探测 cc-switch 失败",
                  })}
                  : {error instanceof Error ? error.message : String(error)}
                </span>
                <Button
                  variant="outline"
                  size="sm"
                  onClick={() => refetch()}
                  className="shrink-0"
                >
                  {t("common.retry", { defaultValue: "重试" })}
                </Button>
              </AlertDescription>
            </Alert>
          ) : !data?.found ? (
            <div className="py-10 text-center text-sm text-muted-foreground">
              {t("importCcs.notFound", {
                defaultValue: "未检测到本机 cc-switch 数据",
              })}
            </div>
          ) : items.length === 0 ? (
            <div className="py-10 text-center text-sm text-muted-foreground">
              {t("importCcs.empty", {
                defaultValue: "cc-switch 中没有可同步的 Claude 渠道",
              })}
            </div>
          ) : (
            <div className="divide-y divide-border/30">
              {items.map((item) => {
                const isChecked = !!selected[item.originalId];
                return (
                  <div
                    key={item.originalId}
                    className={`flex items-start gap-3 px-2 py-2.5 ${
                      item.importable
                        ? "cursor-pointer hover:bg-muted/40"
                        : "opacity-60"
                    }`}
                    onClick={() => toggle(item)}
                  >
                    <Checkbox
                      checked={isChecked}
                      disabled={!item.importable}
                      onCheckedChange={() => toggle(item)}
                      className="mt-0.5"
                      onClick={(e) => e.stopPropagation()}
                    />
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <span className="truncate text-sm font-medium">
                          {item.importedName}
                        </span>
                        <span
                          className={`shrink-0 rounded-md px-1.5 py-0.5 text-[10px] font-medium ${statusBadgeClass(
                            item.status,
                          )}`}
                        >
                          {t(`importCcs.status.${item.status}`, {
                            defaultValue:
                              item.status === "new"
                                ? "新增"
                                : item.status === "update"
                                  ? "更新"
                                  : "无变化",
                          })}
                        </span>
                        {item.importedName !== item.name && (
                          <span className="shrink-0 text-[10px] text-muted-foreground">
                            {t("importCcs.renamedFrom", {
                              defaultValue: "原名 {{name}}",
                              name: item.name,
                            })}
                          </span>
                        )}
                      </div>
                      {item.baseUrl && (
                        <div
                          className="truncate font-mono text-xs text-muted-foreground"
                          title={item.baseUrl}
                        >
                          {item.baseUrl}
                        </div>
                      )}
                      {item.warning && (
                        <div className="mt-0.5 flex items-center gap-1 text-xs text-amber-600 dark:text-amber-400">
                          <AlertTriangle className="h-3 w-3 shrink-0" />
                          {item.warning}
                        </div>
                      )}
                    </div>
                  </div>
                );
              })}
            </div>
          )}
        </div>

        <DialogFooter className="items-center justify-between sm:justify-between">
          <span className="text-xs text-muted-foreground">
            {data?.found && items.length > 0
              ? t("importCcs.selectedHint", {
                  defaultValue: "已选 {{selected}} / 可选 {{total}}",
                  selected: selectedCount,
                  total: selectableCount,
                })
              : null}
          </span>
          <div className="flex gap-2">
            <Button
              variant="outline"
              onClick={() => onOpenChange(false)}
              disabled={isSyncing}
            >
              {t("common.cancel", { defaultValue: "取消" })}
            </Button>
            <Button
              onClick={handleSync}
              disabled={isSyncing || selectedCount === 0}
            >
              {isSyncing ? (
                <>
                  <Loader2 className="mr-1.5 h-4 w-4 animate-spin" />
                  {t("importCcs.syncing", { defaultValue: "同步中..." })}
                </>
              ) : (
                t("importCcs.syncButton", { defaultValue: "同步所选" })
              )}
            </Button>
          </div>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
