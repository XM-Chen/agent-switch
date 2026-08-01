import path from "node:path";
import { configDefaults, defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

export default defineConfig({
  plugins: [react()],
  resolve: {
    alias: {
      "@": path.resolve(__dirname, "./src"),
    },
  },
  test: {
    environment: "jsdom",
    setupFiles: ["./tests/setupGlobals.ts", "./tests/setupTests.ts"],
    globals: true,
    exclude: [
      ...configDefaults.exclude,
      "**/.claude/worktrees/**",
      // 旧 App/外部配置冲突测试覆盖已退役的客户端配置管理产品面。
      // 源码暂留供后续阶段删除，但不再作为独立网关回归契约。
      "tests/integration/App.test.tsx",
      "tests/hooks/useExternalConfigBridge.test.tsx",
    ],
    coverage: {
      reporter: ["text", "lcov"],
    },
  },
});
