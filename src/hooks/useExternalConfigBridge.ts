/**
 * 外部配置检测/冲突事件桥
 *
 * 单例挂载（App 根）。职责：
 * 1. mount 时 hydrate：拉取 `get_external_config_status`，把 `conflict=true` 的项入队。
 * 2. 订阅 `external-config-changed` 事件：
 *    - 始终按 appType 定向失效相关 live 查询（不做全局抖动）。
 *    - conflict=true 时 upsert 冲突队列（同 app 新 generation 覆盖旧项）。
 *    - conflict=false 时移除该 app 队列项（generation ≤ 事件 generation）。
 * 3. 暴露冲突队列（队头供阻塞对话框展示）与出队方法。
 *
 * UI 不缓存受管配置或冲突全文，只持有展示用的 { appType, generation }。
 */

import { useCallback, useEffect, useState } from "react";
import { useQueryClient } from "@tanstack/react-query";
import { useTauriEvent } from "@/hooks/useTauriEvent";
import { proxyApi } from "@/lib/api/proxy";
import type { ExternalConfigChangedPayload } from "@/types/proxy";

export interface ConflictQueueItem {
  appType: string;
  generation: number;
}

export interface UseExternalConfigBridgeResult {
  /** 当前冲突队列（按事件先后）。队头交给阻塞对话框处理。 */
  conflictQueue: ConflictQueueItem[];
  /** 队头冲突项（无则 undefined）。 */
  currentConflict: ConflictQueueItem | undefined;
  /** 处理完成后从队列移除指定 app 项。 */
  dequeueConflict: (appType: string) => void;
}

export function useExternalConfigBridge(): UseExternalConfigBridgeResult {
  const queryClient = useQueryClient();
  const [conflictQueue, setConflictQueue] = useState<ConflictQueueItem[]>([]);

  // 按 appType 定向失效：只刷新该模块相关 live 查询，避免全局抖动。
  const invalidateForApp = useCallback(
    (appType: string) => {
      queryClient.invalidateQueries({ queryKey: ["proxyTakeoverStatus"] });
      queryClient.invalidateQueries({ queryKey: ["externalConfigStatus"] });
      queryClient.invalidateQueries({ queryKey: ["providers", appType] });
      queryClient.invalidateQueries({ queryKey: ["appProxyConfig", appType] });
    },
    [queryClient],
  );

  // upsert：同 app 已在队列则用新 generation 覆盖，否则追加到队尾。
  const upsertConflict = useCallback((appType: string, generation: number) => {
    setConflictQueue((prev) => {
      const existingIndex = prev.findIndex((item) => item.appType === appType);
      if (existingIndex >= 0) {
        const next = [...prev];
        next[existingIndex] = { appType, generation };
        return next;
      }
      return [...prev, { appType, generation }];
    });
  }, []);

  // 移除：仅当队列中该 app 的 generation ≤ 事件 generation 才移除（避免误清更新的冲突）。
  const removeResolvedConflict = useCallback(
    (appType: string, generation: number) => {
      setConflictQueue((prev) =>
        prev.filter(
          (item) =>
            !(item.appType === appType && item.generation <= generation),
        ),
      );
    },
    [],
  );

  // 用户处理完成后显式出队（无视 generation）。
  const dequeueConflict = useCallback((appType: string) => {
    setConflictQueue((prev) =>
      prev.filter((item) => item.appType !== appType),
    );
  }, []);

  // mount hydrate：把已存在的冲突项入队。
  useEffect(() => {
    let disposed = false;
    void (async () => {
      try {
        const statuses = await proxyApi.getExternalConfigStatus();
        if (disposed) return;
        const conflicts = statuses.filter((s) => s.conflict);
        if (conflicts.length === 0) return;
        setConflictQueue((prev) => {
          const next = [...prev];
          for (const s of conflicts) {
            const existingIndex = next.findIndex(
              (item) => item.appType === s.appType,
            );
            if (existingIndex >= 0) {
              next[existingIndex] = {
                appType: s.appType,
                generation: s.generation,
              };
            } else {
              next.push({ appType: s.appType, generation: s.generation });
            }
          }
          return next;
        });
      } catch (error) {
        console.error(
          "[useExternalConfigBridge] Failed to hydrate external config status",
          error,
        );
      }
    })();
    return () => {
      disposed = true;
    };
  }, []);

  useTauriEvent<ExternalConfigChangedPayload>(
    "external-config-changed",
    (payload) => {
      invalidateForApp(payload.appType);
      if (payload.conflict) {
        upsertConflict(payload.appType, payload.generation);
      } else {
        removeResolvedConflict(payload.appType, payload.generation);
      }
    },
  );

  return {
    conflictQueue,
    currentConflict: conflictQueue[0],
    dequeueConflict,
  };
}
