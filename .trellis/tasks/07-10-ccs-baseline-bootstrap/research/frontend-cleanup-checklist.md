# 前端清理检查清单

## 状态：正在执行 Phase 2.1 前端裁剪

### 已删除的文件

#### Components
- [x] `src/components/CodexOauthQuotaFooter.tsx`
- [x] `src/components/CopilotQuotaFooter.tsx`
- [x] `src/components/agents/` (整个目录)
- [x] `src/components/openclaw/` (整个目录)
- [x] `src/components/workspace/` (整个目录)
- [x] `src/components/hermes/` (整个目录)
- [x] `src/components/skills/` (整个目录)
- [x] `src/components/prompts/` (整个目录)
- [x] `src/components/providers/forms/GeminiCommonConfigModal.tsx`
- [x] `src/components/providers/forms/GeminiConfigEditor.tsx`
- [x] `src/components/providers/forms/GeminiConfigSections.tsx`
- [x] `src/components/providers/forms/GeminiFormFields.tsx`
- [x] `src/components/providers/forms/HermesFormFields.tsx`
- [x] `src/components/providers/forms/OmoFormFields.tsx`
- [x] `src/components/providers/forms/OpenClawFormFields.tsx`
- [x] `src/components/providers/forms/OpenCodeFormFields.tsx`
- [x] `src/components/providers/forms/CopilotAuthSection.tsx`
- [x] `src/components/providers/forms/CodexCommonConfigModal.tsx`
- [x] `src/components/providers/forms/CodexConfigEditor.tsx`
- [x] `src/components/providers/forms/CodexConfigSections.tsx`
- [x] `src/components/providers/forms/CodexFormFields.tsx`
- [x] `src/components/providers/forms/CodexOAuthSection.tsx`
- [x] `src/components/settings/CodexAuthSettings.tsx`
- [x] `src/components/settings/SkillStorageLocationSettings.tsx`
- [x] `src/components/settings/SkillSyncMethodSettings.tsx`
- [x] `src/components/settings/WebdavSyncSection.tsx`

#### Hooks
- [x] `src/hooks/useHermes.ts`
- [x] `src/hooks/useOpenClaw.ts`
- [x] `src/hooks/usePromptActions.ts`
- [x] `src/hooks/useSkills.ts`
- [x] `src/hooks/useSkills.helpers.ts`

#### API
- [x] `src/lib/api/copilot.ts`
- [x] `src/lib/api/hermes.ts`
- [x] `src/lib/api/omo.ts`
- [x] `src/lib/api/openclaw.ts`
- [x] `src/lib/api/prompts.ts`
- [x] `src/lib/api/skills.ts`
- [x] `src/lib/api/workspace.ts`

#### Types
- [x] `src/types/omo.ts`

### 需要修复的导入引用

#### `src/App.tsx`
- [ ] Line 41: `@/hooks/useOpenClaw` → 需注释或移除
- [ ] Line 42: `@/hooks/useHermes` → 需注释或移除
- [ ] Line 43: `@/lib/api/hermes` → 需注释或移除
- [ ] Line 49: `@/hooks/useSkills` → 需注释或移除
- [ ] Line 73: `@/components/prompts/PromptPanel` → 需注释或移除
- [ ] Line 78: `@/components/skills/SkillsPage` → 需注释或移除
- [ ] Line 79: `@/components/skills/UnifiedSkillsPanel` → 需注释或移除
- [ ] Line 82: `@/components/agents/AgentsPanel` → 需注释或移除
- [ ] Line 91: `@/components/workspace/WorkspaceFilesPanel` → 需注释或移除
- [ ] Line 92-95: `@/components/openclaw/*` → 需注释或移除
- [ ] Line 96: `@/components/hermes/HermesMemoryPanel` → 需注释或移除
- [ ] 相关 View 类型和逻辑需要清理

#### 其他文件
- [ ] `src/components/providers/forms/ClaudeDesktopProviderForm.tsx`
- [ ] `src/components/providers/forms/ClaudeFormFields.tsx`
- [ ] `src/components/providers/forms/ProviderForm.tsx`
- [ ] `src/components/providers/ProviderCard.tsx`
- [ ] `src/components/providers/ProviderList.tsx`
- [ ] `src/components/settings/AuthCenterPanel.tsx`
- [ ] `src/components/settings/SettingsPage.tsx`
- [ ] `src/components/providers/forms/helpers/opencodeFormUtils.ts`
- [ ] `src/components/providers/forms/hooks/useOmoDraftState.ts`

### 下一步
1. 修复 `App.tsx` 的导入和类型
2. 逐一修复其他文件的导入引用
3. 确保 TypeScript 编译通过
4. 运行 `pnpm typecheck` 验证
