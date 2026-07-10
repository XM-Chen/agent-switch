# 产品身份、数据目录与更新

## 目标身份映射

| 维度 | ccs v3.16.5 | Agent Switch 目标 |
|---|---|---|
| productName | CC Switch | Agent-Switch |
| identifier | `com.ccswitch.desktop` | `com.agent-switch.app` |
| npm/Cargo/lib | `cc-switch` / `cc_switch_lib` | `agent-switch` / `agent_switch_lib` |
| version | 3.16.5 | 0.3.0 |
| data root | `~/.cc-switch` | `~/.agent-switch` |
| DB/log | `cc-switch.db` / `cc-switch.log` | `agent-switch.db` / `agent-switch.log` |
| Deep Link | `ccswitch://` | `agentswitch://` |
| updater | ccs GitHub + ccs pubkey | `XM-Chen/agent-switch` + 自有公钥 |

当前 Tauri 身份和 updater 集中在 `src-tauri/tauri.conf.json:1-68`。三处版本必须同步：`package.json`、`src-tauri/Cargo.toml`、`src-tauri/tauri.conf.json`。

## 数据根

所有路径从单一 `get_app_config_dir()` 或常量派生，禁止散落硬编码。身份任务必须扫描并改名：

- DB、backups、skills/skill-backups、logs/crash；
- panic hook、env manager、sync remote root、test home；
- Windows legacy fallback、portable/store；
- user-agent/export header/manifest。

新产品不得读取 `~/.cc-switch` 或旧 Agent Switch DB。新空产品库仍要先保护性导入现有 `~/.claude/settings.json`；这是 live 保护，不是产品 DB 迁移。

## Deep Link

只注册 `agentswitch`；同步修改：

- Tauri scheme；
- backend parser/runtime listener/starts_with；
- frontend placeholder/help；
- `source_protocol`；
- provider/prompt/mcp/skill 四类导入测试。

不注册 `ccswitch://` 为系统处理程序。应用内粘贴兼容不默认实现，需另有明确需求。

## Windows MSI 身份

Agent Switch 0.3.0 要替代旧 Agent Switch、与 CC Switch 共存：

- identifier 使用 `com.agent-switch.app`；
- 在旧 Agent Switch 身份上通过 Tauri inspect 取得默认 WiX UpgradeCode；
- 在新配置显式固定 `bundle.windows.wix.upgradeCode`；
- 在隔离 Windows 测试旧 Agent Switch → 0.3.0 升级，以及 CC Switch 并存。

不能假定改 identifier 自动保持 MSI 升级线。

## Updater 全链路

当前 ccs updater 配置在 `tauri.conf.json:35-66`，`createUpdaterArtifacts=true`。目标：

1. endpoint = `https://github.com/XM-Chen/agent-switch/releases/latest/download/latest.json`；
2. `pubkey` = Agent Switch 公钥内容；
3. 私钥只从安全环境变量/secret 注入，绝不入库；
4. release 只构建 Windows x86_64；
5. 生成 MSI + `.sig`；
6. `latest.json` 含 version、`windows-x86_64.url`、signature（`.sig` 内容）；
7. 安装客户端 check→download→verify→restart 端到端。

Tauri updater 涉及 config、capability、plugin 注册、前端 UpdateContext/API、安装前代理/tray/window 清理和 release workflow；不能只改 endpoint。

未经单独授权不 push tag、不创建 GitHub Release、不发布资产。

## 来源归属与品牌

删除 CC Switch 品牌/图标/赞助/联盟/社区内容，但保留 MIT LICENSE 和 Jason Young 原版权；中文 README/关于页注明基于 CC Switch v3.16.5 修改。
