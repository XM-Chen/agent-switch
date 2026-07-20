/**
 * 代理服务状态管理 Hook
 */

import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { invoke } from "@tauri-apps/api/core";
import { toast } from "sonner";
import { useTranslation } from "react-i18next";
import type {
  ProxyStatus,
  ProxyServerInfo,
  ProxyTakeoverStatus,
  ProxyRouteMode,
} from "@/types/proxy";
import { extractErrorMessage } from "@/utils/errorUtils";
import { parseProxyInvokeError } from "@/utils/proxyError";

// 七模块规范 app_type → 展示标签
const APP_LABELS: Record<string, string> = {
  claude: "Claude",
  "claude-desktop": "Claude Desktop",
  codex: "Codex",
  gemini: "Gemini",
  opencode: "OpenCode",
  openclaw: "OpenClaw",
  hermes: "Hermes",
};

const appLabelFor = (appType: string): string =>
  APP_LABELS[appType] ?? appType;

/**
 * 代理服务状态管理
 */
export function useProxyStatus() {
  const queryClient = useQueryClient();
  const { t } = useTranslation();

  // 查询状态（自动轮询）
  const { data: status, isLoading } = useQuery({
    queryKey: ["proxyStatus"],
    queryFn: () => invoke<ProxyStatus>("get_proxy_status"),
    // 仅在服务运行时轮询
    refetchInterval: (query) => (query.state.data?.running ? 2000 : false),
    // 保持之前的数据，避免闪烁
    placeholderData: (previousData) => previousData,
  });

  // 查询各应用接管状态
  const { data: takeoverStatus } = useQuery({
    queryKey: ["proxyTakeoverStatus"],
    queryFn: () => invoke<ProxyTakeoverStatus>("get_proxy_takeover_status"),
    placeholderData: (previousData) => previousData,
  });

  // 启动服务器（总开关：仅启动服务，不接管）
  const startProxyServerMutation = useMutation({
    mutationFn: () => invoke<ProxyServerInfo>("start_proxy_server"),
    onSuccess: (info) => {
      toast.success(
        t("proxy.server.started", {
          address: info.address,
          port: info.port,
          defaultValue: `代理服务已启动 - ${info.address}:${info.port}`,
        }),
        { closeButton: true },
      );
      queryClient.invalidateQueries({ queryKey: ["proxyStatus"] });
    },
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("proxy.server.startFailed", {
          detail,
          defaultValue: `启动代理服务失败: ${detail}`,
        }),
      );
    },
  });

  // 停止服务器（仅停止服务，不改写/恢复其它应用接管状态）
  const stopProxyServerMutation = useMutation({
    mutationFn: () => invoke("stop_proxy_server"),
    onSuccess: () => {
      toast.success(
        t("proxy.server.stopped", {
          defaultValue: "代理服务已停止",
        }),
        { closeButton: true },
      );
      queryClient.invalidateQueries({ queryKey: ["proxyStatus"] });
    },
    onError: (error: unknown) => {
      const stopError = parseProxyInvokeError(error);
      // 仍有 proxy 路由模块时后端拒绝停止：展示模块列表，不改任何模块状态
      if (stopError?.code === "proxyRoutesActive") {
        const modules = stopError.modules
          .map((m) => appLabelFor(m))
          .join("、");
        toast.error(
          t("proxy.server.stopBlockedByProxyRoutes", {
            modules,
            defaultValue: `以下模块正走本地代理，请先切换为 direct 或关闭接管再停止网关：${modules}`,
          }),
          { duration: 6000 },
        );
        return;
      }
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("proxy.server.stopFailed", {
          detail,
          defaultValue: `停止代理服务失败: ${detail}`,
        }),
      );
    },
  });

  // 旧调用名兼容：同样执行受保护的纯网关停止。
  const stopWithRestoreMutation = useMutation({
    mutationFn: () => invoke("stop_proxy_server"),
    onSuccess: () => {
      toast.success(
        t("proxy.server.stopped", {
          defaultValue: "代理服务已停止",
        }),
        { closeButton: true },
      );
      queryClient.invalidateQueries({ queryKey: ["proxyStatus"] });
      queryClient.invalidateQueries({ queryKey: ["proxyTakeoverStatus"] });
      // 彻底删除所有供应商健康状态缓存（后端已清空数据库记录）
      queryClient.removeQueries({ queryKey: ["providerHealth"] });
      // 彻底删除所有熔断器统计缓存（代理停止后熔断器状态已重置）
      queryClient.removeQueries({ queryKey: ["circuitBreakerStats"] });
      // 注意：故障转移队列和开关状态会保留，不需要刷新
    },
    onError: (error: unknown) => {
      const stopError = parseProxyInvokeError(error);
      if (stopError?.code === "proxyRoutesActive") {
        const modules = stopError.modules
          .map((m) => appLabelFor(m))
          .join("、");
        toast.error(
          t("proxy.server.stopBlockedByProxyRoutes", {
            modules,
            defaultValue: `以下模块正走本地代理，请先切换为 direct 或关闭接管再停止网关：${modules}`,
          }),
          { duration: 6000 },
        );
        return;
      }
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("proxy.stopWithRestoreFailed", {
          detail,
          defaultValue: `停止失败: ${detail}`,
        }),
      );
    },
  });

  // 按应用开启/关闭接管
  // 开启时携带 routeMode（缺省由后端按 direct 处理）。
  const setTakeoverForAppMutation = useMutation({
    mutationFn: ({
      appType,
      enabled,
      routeMode,
    }: {
      appType: string;
      enabled: boolean;
      routeMode?: ProxyRouteMode;
    }) =>
      invoke("set_proxy_takeover_for_app", { appType, enabled, routeMode }),
    onSuccess: (_data, variables) => {
      const appLabel = appLabelFor(variables.appType);

      toast.success(
        variables.enabled
          ? t("proxy.takeover.enabled", {
              app: appLabel,
              defaultValue: `已接管 ${appLabel} 配置`,
            })
          : t("proxy.takeover.disabled", {
              app: appLabel,
              defaultValue: `已恢复 ${appLabel} 配置`,
            }),
        { closeButton: true },
      );

      queryClient.invalidateQueries({ queryKey: ["proxyStatus"] });
      queryClient.invalidateQueries({ queryKey: ["proxyTakeoverStatus"] });
    },
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("proxy.takeover.failed", {
          detail,
          defaultValue: `操作失败: ${detail}`,
        }),
      );
    },
  });

  // 代理模式切换供应商（热切换）
  const switchProxyProviderMutation = useMutation({
    mutationFn: ({
      appType,
      providerId,
    }: {
      appType: string;
      providerId: string;
    }) => invoke("switch_proxy_provider", { appType, providerId }),
    onSuccess: () => {
      queryClient.invalidateQueries({ queryKey: ["proxyStatus"] });
    },
    onError: (error: Error) => {
      const detail =
        extractErrorMessage(error) ||
        t("common.unknown", { defaultValue: "未知错误" });
      toast.error(
        t("proxy.switchFailed", {
          error: detail,
          defaultValue: `切换失败: ${detail}`,
        }),
      );
    },
  });

  // 检查是否运行中
  const checkRunning = async () => {
    try {
      return await invoke<boolean>("is_proxy_running");
    } catch {
      return false;
    }
  };

  // 检查接管状态
  const checkTakeoverActive = async () => {
    try {
      return await invoke<boolean>("is_live_takeover_active");
    } catch {
      return false;
    }
  };

  return {
    status,
    isLoading,
    isRunning: status?.running || false,
    takeoverStatus,
    isTakeoverActive: takeoverStatus
      ? Object.values(takeoverStatus).some((module) => module.takeoverEnabled)
      : false,

    // 启动/停止（总开关）
    startProxyServer: startProxyServerMutation.mutateAsync,
    stopProxyServer: stopProxyServerMutation.mutateAsync,
    stopWithRestore: stopWithRestoreMutation.mutateAsync,

    // 按应用接管开关
    setTakeoverForApp: setTakeoverForAppMutation.mutateAsync,

    // 代理模式下切换供应商
    switchProxyProvider: switchProxyProviderMutation.mutateAsync,

    // 状态检查
    checkRunning,
    checkTakeoverActive,

    // 加载状态
    isStarting: startProxyServerMutation.isPending,
    isStoppingServer: stopProxyServerMutation.isPending,
    isStopping: stopWithRestoreMutation.isPending,
    isPending:
      startProxyServerMutation.isPending ||
      stopProxyServerMutation.isPending ||
      stopWithRestoreMutation.isPending ||
      setTakeoverForAppMutation.isPending,
  };
}
