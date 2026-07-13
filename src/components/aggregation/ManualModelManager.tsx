/**
 * 手动加/删模型子界面（C4，R3）
 *
 * 为队列内某个上游手动补录 model id，并列出该上游全部缓存行（区分 fetched/manual）。
 * manual 行带标记、可删；fetched 行只读。手动模型不受自动刷新删除影响（D6）。
 */

import { useState } from "react";
import { useTranslation } from "react-i18next";
import { Trash2, Loader2, Plus } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Badge } from "@/components/ui/badge";
import type { AppId } from "@/lib/api";
import {
  useProviderModelsQuery,
  useAddManualModel,
  useRemoveManualModel,
} from "@/lib/query/aggregation";
import { toast } from "sonner";
import { formatFetchedAt } from "./aggregationHelpers";

interface ManualModelManagerProps {
  appId: AppId;
  providerId: string;
}

export function ManualModelManager({
  appId,
  providerId,
}: ManualModelManagerProps) {
  const { t } = useTranslation();
  const [draft, setDraft] = useState("");
  const { data: models = [], isLoading } = useProviderModelsQuery(
    appId,
    providerId,
  );
  const addManual = useAddManualModel();
  const removeManual = useRemoveManualModel();

  const handleAdd = () => {
    const modelId = draft.trim();
    if (!modelId) {
      toast.error(
        t("aggregation.manualModel.emptyId", {
          defaultValue: "请输入模型 id",
        }),
      );
      return;
    }
    // 明显非法：含空白字符（模型 id 不应含空格）。
    if (/\s/.test(modelId)) {
      toast.error(
        t("aggregation.manualModel.invalidId", {
          defaultValue: "模型 id 不能包含空格",
        }),
      );
      return;
    }
    // 重复：已存在同名行。
    if (models.some((m) => m.modelId === modelId)) {
      toast.error(
        t("aggregation.manualModel.duplicate", {
          defaultValue: "该模型 id 已存在",
        }),
      );
      return;
    }
    addManual.mutate(
      { appType: appId, providerId, modelId },
      { onSuccess: () => setDraft("") },
    );
  };

  const handleRemove = (modelId: string) => {
    removeManual.mutate({ appType: appId, providerId, modelId });
  };

  return (
    <div className="space-y-3">
      <div className="flex gap-2">
        <Input
          value={draft}
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter") {
              e.preventDefault();
              handleAdd();
            }
          }}
          placeholder={t("aggregation.manualModel.placeholder", {
            defaultValue: "输入要补录的模型 id",
          })}
          className="flex-1"
        />
        <Button
          variant="outline"
          size="sm"
          onClick={handleAdd}
          disabled={addManual.isPending}
        >
          {addManual.isPending ? (
            <Loader2 className="h-4 w-4 animate-spin" />
          ) : (
            <Plus className="h-4 w-4" />
          )}
          {t("aggregation.manualModel.add", { defaultValue: "添加" })}
        </Button>
      </div>

      <p className="text-xs text-muted-foreground">
        {t("aggregation.manualModel.hint", {
          defaultValue: "手动模型不受自动刷新删除影响，仅可手动移除。",
        })}
      </p>

      {isLoading ? (
        <div className="flex items-center gap-2 text-sm text-muted-foreground">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t("common.loading", { defaultValue: "加载中…" })}
        </div>
      ) : models.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          {t("aggregation.manualModel.empty", {
            defaultValue: "该上游暂无缓存模型，可手动补录或等待自动刷新。",
          })}
        </p>
      ) : (
        <ul className="space-y-1.5">
          {models.map((model) => (
            <li
              key={`${model.modelId}:${model.source}`}
              className="flex items-center justify-between gap-2 rounded-md border border-border bg-card/40 px-3 py-1.5 text-sm"
            >
              <div className="flex min-w-0 items-center gap-2">
                <span className="truncate font-mono text-xs">
                  {model.modelId}
                </span>
                {model.source === "manual" ? (
                  <Badge variant="secondary" className="shrink-0">
                    {t("aggregation.manualModel.manualTag", {
                      defaultValue: "手动",
                    })}
                  </Badge>
                ) : (
                  <Badge variant="outline" className="shrink-0">
                    {t("aggregation.manualModel.fetchedTag", {
                      defaultValue: "自动",
                    })}
                  </Badge>
                )}
              </div>
              <div className="flex shrink-0 items-center gap-2">
                {model.fetchedAt > 0 && (
                  <span className="text-[11px] text-muted-foreground">
                    {formatFetchedAt(model.fetchedAt)}
                  </span>
                )}
                {model.source === "manual" && (
                  <Button
                    variant="ghost"
                    size="icon"
                    className="h-7 w-7 text-muted-foreground hover:text-destructive"
                    onClick={() => handleRemove(model.modelId)}
                    disabled={removeManual.isPending}
                    title={t("aggregation.manualModel.remove", {
                      defaultValue: "删除手动模型",
                    })}
                  >
                    <Trash2 className="h-3.5 w-3.5" />
                  </Button>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}
