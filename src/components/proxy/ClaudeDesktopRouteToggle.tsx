/**
 * Claude Desktop 顶栏开关。
 *
 * C4-D2：收口为「模块接管开关」，不再启停本地网关。
 * 直接复用统一的 ProxyToggle，固定 activeApp 为 claude-desktop。
 */

import { ProxyToggle } from "@/components/proxy/ProxyToggle";

interface ClaudeDesktopRouteToggleProps {
  className?: string;
}

export function ClaudeDesktopRouteToggle({
  className,
}: ClaudeDesktopRouteToggleProps) {
  return <ProxyToggle className={className} activeApp="claude-desktop" />;
}
