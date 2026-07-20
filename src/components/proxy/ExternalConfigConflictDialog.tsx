/**
 * 外部配置冲突对话框（阻塞模态 + 单冲突队列）
 *
 * 契约（C4-D3 / R13/R14）：
 * - 一次只处理一个模块（队头）；不可忽略关闭（点遮罩/Esc 不关）。
 * - 仅两条路径：接受外部更改 / 使用 Agent-Switch 配置覆盖。
 * - 后端成功后才出队并刷新；失败展示原因并保持对话框。
 * - 前端不缓存受管配置或冲突全文，只显示模块名。
 */

import { useState } from "react";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Button } from "@/components/ui/button";
import { AlertTriangle, Loader2 } from "lucide-react";
import { useTranslation } from "react-i18next";
import { extractErrorMessage } from "@/utils/errorUtils";
import {
  useAcceptExternalConfigChange,
  useRejectExternalConfigChange,
} from "@/lib/query/proxy";
import type { ConflictQueueItem } from "@/hooks/useExternalConfigBridge";

const APP_LABELS: Record<string, string> = {
  claude: "Claude",
  "claude-desktop": "Claude Desktop",
  codex: "Codex",
  gemini: "Gemini",
  opencode: "OpenCode",
  openclaw: "OpenClaw",
  hermes: "Hermes",
};

interface ExternalConfigConflictDialogProps {
  conflict: ConflictQueueItem | undefined;
  /** 后端成功处理后调用，把该 app 从队列出队。 */
  onResolved: (appType: string) => void;
}

export function ExternalConfigConflictDialog({
  conflict,
  onResolved,
}: ExternalConfigConflictDialogProps) {
  const { t } = useTranslation();
  const acceptMutation = useAcceptExternalConfigChange();
  const rejectMutation = useRejectExternalConfigChange();
  const [errorMessage, setErrorMessage] = useState<string | null>(null);

  const open = Boolean(conflict);
  const appLabel = conflict
    ? (APP_LABELS[conflict.appType] ?? conflict.appType)
    : "";
  const isPending = acceptMutation.isPending || rejectMutation.isPending;

  const handleAccept = async () => {
    if (!conflict) return;
    setErrorMessage(null);
    try {
      await acceptMutation.mutateAsync({
        appType: conflict.appType,
        generation: conflict.generation,
      });
      onResolved(conflict.appType);
    } catch (error) {
      // 失败保留冲突态与对话框，展示后端原因（例如无法解析路由）。
      setErrorMessage(
        extractErrorMessage(error) ||
          t("common.unknown", { defaultValue: "未知错误" }),
      );
    }
  };

  const handleReject = async () => {
    if (!conflict) return;
    setErrorMessage(null);
    try {
      await rejectMutation.mutateAsync({
        appType: conflict.appType,
        generation: conflict.generation,
      });
      onResolved(conflict.appType);
    } catch (error) {
      setErrorMessage(
        extractErrorMessage(error) ||
          t("common.unknown", { defaultValue: "未知错误" }),
      );
    }
  };

  return (
    <Dialog open={open}>
      <DialogContent
        zIndex="alert"
        className="max-w-md"
        // 阻塞：Esc 不关闭
        onEscapeKeyDown={(e) => e.preventDefault()}
      >
        <DialogHeader className="space-y-3 border-b-0 bg-transparent pb-0">
          <DialogTitle className="flex items-center gap-2 text-lg font-semibold">
            <AlertTriangle className="h-5 w-5 text-amber-500" />
            {t("proxy.externalConfig.conflictTitle", {
              app: appLabel,
              defaultValue: `${appLabel} 配置被外部修改`,
            })}
          </DialogTitle>
          <DialogDescription className="whitespace-pre-line text-sm leading-relaxed">
            {t("proxy.externalConfig.conflictMessage", {
              app: appLabel,
              defaultValue: `检测到 ${appLabel} 的配置在 Agent-Switch 接管期间被外部修改。请选择保留外部更改，或用 Agent-Switch 当前配置覆盖。`,
            })}
          </DialogDescription>
        </DialogHeader>

        {errorMessage ? (
          <div className="mx-6 rounded-md border border-destructive/40 bg-destructive/10 px-3 py-2 text-sm text-destructive">
            {errorMessage}
          </div>
        ) : null}

        <DialogFooter className="flex gap-2 border-t-0 bg-transparent pt-2 sm:justify-end">
          <Button
            variant="outline"
            onClick={() => void handleReject()}
            disabled={isPending}
          >
            {rejectMutation.isPending ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : null}
            {t("proxy.externalConfig.rejectAction", {
              defaultValue: "使用 Agent-Switch 配置覆盖",
            })}
          </Button>
          <Button
            variant="default"
            onClick={() => void handleAccept()}
            disabled={isPending}
          >
            {acceptMutation.isPending ? (
              <Loader2 className="mr-2 h-4 w-4 animate-spin" />
            ) : null}
            {t("proxy.externalConfig.acceptAction", {
              defaultValue: "接受外部更改",
            })}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
