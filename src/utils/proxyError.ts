import type { ProxyStopError } from "@/types/proxy";

/**
 * 解析后端返回的结构化代理错误。
 *
 * Tauri 可能把结构化错误序列化为 JSON 字符串，也可能已是对象。
 * 这里兼容两种形态，仅在能识别出 `code` 字段时返回 ProxyStopError，
 * 否则返回 null 交给通用 extractErrorMessage 处理。
 */
export function parseProxyInvokeError(error: unknown): ProxyStopError | null {
  if (!error) return null;

  let candidate: unknown = error;

  // Error 实例：message 可能是 JSON 字符串
  if (error instanceof Error) {
    candidate = error.message;
  }

  // 字符串：尝试 JSON.parse
  if (typeof candidate === "string") {
    const trimmed = candidate.trim();
    if (!trimmed.startsWith("{")) return null;
    try {
      candidate = JSON.parse(trimmed);
    } catch {
      return null;
    }
  }

  if (!candidate || typeof candidate !== "object") return null;

  const obj = candidate as Record<string, unknown>;
  const code = obj.code;
  if (typeof code !== "string") return null;

  const message = typeof obj.message === "string" ? obj.message : "";
  const modules = Array.isArray(obj.modules)
    ? obj.modules.filter((m): m is string => typeof m === "string")
    : [];

  return { code, message, modules };
}
