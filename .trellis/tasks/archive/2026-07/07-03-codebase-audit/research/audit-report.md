# agent-switch 代码库审计报告

> 审计日期：2026-07-03
> 审计范围：10 个子系统由独立 finder 全读覆盖，2 名 verifier 对每条 P0/P1 做对抗验证后产出 survives 标记。
> 本报告对全部 verified findings 做去重、跨层关联与严重度排序。

---

## 1. 执行摘要

本轮审计 finder 阶段共产出 **71** 条原始发现，经去重（1 条跨 finder 重复：api.ts:423 与 api.ts:45 同源）与证伪（1 条 survives=false）后，**69 条存活**进入主表，分布如下：

| 严重度 | 计数 | 说明 |
|--------|------|------|
| P0 | 0 | 无 P0 提交且存活 |
| P1 | 5 | 确认存活的高优先级正确性缺陷 |
| P2 | 24 | 中等正确性/契约偏离（含 3 条由原 P1 降级） |
| P3 | 40 | 代码质量/死代码/轻微一致性（含 1 条由原 P2 降级观察项） |

- **确认存活的 P0/P1**：5 条（全部为 P1，无 P0 上调）。
- **被证伪/降级的原 P0/P1**：4 条。其中 1 条原 P1 被证伪（survives=false，anthropic_openai.rs:943），3 条原 P1 经 verifier 二审降级为 P2（route_settings 残留、ui_settings 丢弃、生产日志过滤）。另 1 条跨 finder 重复（api.ts:45 并入 api.ts:423）作去重处理，不计入 dropped。
- **无 P0**：所有缺陷均无崩溃、数据丢失不可恢复或安全关键路径致命数据损坏，故最高定级为 P1。

一句话结论：**代码库整体功能链路可用，但存在 5 条高优先级缺陷集中于「跨协议流式工具调用 wire format 残缺」「媒体 passthrough 端到端失败」「重复 OAuth 登录路径断裂」「流式链路测试前端默认即坏」「Dashboard 错误态静默误导」；另有若干导入/导出与过滤语义的契约偏离已被降级处理。**

---

## 2. P0 发现表

无确认存活的 P0。

---

## 3. P1 发现表

### P1-1 Passthrough（images/audio）在 multipart/form-data 请求体上端到端失败

| 项 | 内容 |
|----|------|
| 标题 | Passthrough (images/audio) breaks on multipart/form-data bodies — model_mapper runs before passthrough and requires a JSON 'model' key |
| 文件:行 | `src-tauri/src/http/proxy/mod.rs:304` |
| 触发 | 客户端对 `/v1/images/edits`、`/v1/images/variations`、`/v1/audio/transcriptions`、`/v1/audio/translations` 等 passthrough 能力发 multipart/form-data 请求。`to_bytes` 收集到二进制后 `serde_json::from_slice` 失败，`req_json=Value::Null`。failover 主循环在 `mod.rs:208` 无条件先执行 `model_mapper.resolve_and_rewrite(&mut req_body_clone)`（即 Null body），`model_mapper.rs:69-73` 要求 `body['model']`，Null.get 返回 None → `Err("请求体缺少 model 字段")`，`mod.rs:216-224` 记录失败并 `continue`。每个候选端点都拿到同一 Null body 重复失败，耗尽后 `mod.rs:740` 返回 502。is_passthrough 的原始二进制转发分支（`mod.rs:304-306`）在 model 映射之后，本路径永远不可达。 |
| 影响 | media passthrough 对 multipart 端点恒定以 502 "请求体缺少 model 字段" 失败，audio/image 的 transcription/translation/edit/variations 工作流被破坏，错误信息误导诊断。无安全/数据丢失维度，客户端可绕过直连上游 URL 故非 P0。 |
| 修复方向 | is_passthrough 时在 model 映射前跳过 `resolve_and_rewrite`，或从 multipart form field / endpoint 默认模型推导 `upstream_model`，再进入原始 body 转发。 |
| 验证摘要 | 2 名 verifier 均确认 real，逐行追踪 mod.rs:101-102（Null fallback）、capability.rs:36-39（images/audio 映射）、mod.rs:208（无条件 resolve）、model_mapper.rs:69-73（缺 model 报错）、mod.rs:216-224（record_failure+continue）、mod.rs:740（502）。无反驳。维持 P1。 |

### P1-2 ChatToAnthropic 所有 content_block 未发送 content_block_stop（tool_use input JSON 无法 finalize）

| 项 | 内容 |
|----|------|
| 标题 | ChatToAnthropic 所有 content_block 未发送 content_block_stop（tool_use 块的 input JSON 无法 finalize） |
| 文件:行 | `src-tauri/src/services/translator/anthropic_openai.rs:968` |
| 触发 | 入站协议 anthropic、上游 openai-chat。上游流以 `finish_reason` 结束、随后发 `[DONE]`。tool_use 块已发过 `content_block_start` 与若干 `input_json_delta`，但转换器在 finish_reason 路径（1000-1012）与 [DONE] 路径（866-868）均不发对应 `content_block_stop`。全仓 grep 确认 `content_block_stop` 仅出现在反向 AnthropicToChat 分支（576-579，纯跳过）。 |
| 影响 | 符合 Anthropic 流式协议的客户端需 `content_block_stop` 时才将 `input_json_delta` 拼接体 finalize 为 JSON；缺失该事件使工具参数无法闭合/finalize，工具调用对客户端不可用。这是 Claude Code 跨协议流式+工具调用的核心场景，每次流式工具调用都会命中。 |
| 修复方向 | 在 tool_use 块的 arguments 流结束（检测到新块/finish_reason/message_stop 时）为每个已打开的 tool_use 发 `content_block_stop`；可保留 `acc` 在 stop 时输出最终 input JSON。 |
| 验证摘要 | 2 名 verifier 均确认 real，独立追踪 ChatToAnthropicTranslator::translate_stream_line（854-1019）与全仓 grep，确认无 `content_block_stop` 在前向路径产生。维持 P1。 |

> 关联：与原 P1（anthropic_openai.rs:943，survives=false，见 §6）描述同一前向流式 wire-format 残缺问题族，但 943 的「text 无 content_block_start」与「index 0 冲突」子项被证伪（见 §6），仅保留本条的 `content_block_stop` 缺失子项存活。

### P1-3 重复 OAuth 登录同一 ChatGPT 账号因 PRIMARY KEY 冲突返回 500，新 token 被丢弃

| 项 | 内容 |
|----|------|
| 标题 | 重复 OAuth 登录同一 ChatGPT 账号因 accounts PRIMARY KEY 冲突而失败，新 token 被丢弃 |
| 文件:行 | `src-tauri/src/services/codex_oauth.rs:294` |
| 触发 | 用户已 OAuth 登录过 ChatGPT 账号 A（accounts 表已有 id=chatgptAccountId 的行）。再次走 `/api/auth/codex/login` → `handle_callback` 解析 id_token JWT 中的 `chatgptAccountId`（codex_oauth.rs:234-238, 393-424，同一账号稳定 id），`account_id_str` 复用旧 id（265-267），`NewAccount.id` 等于已存行（285-286）。`accounts::create`（dao/accounts.rs:108-122）为纯 INSERT 无 ON CONFLICT，第二次插入触发 `UNIQUE constraint failed: accounts.id`，`create` 返回 Err，`codex_oauth.rs:294` 捕获后返回 HTTP 500 `db_save_failed`，`cleanup_session` 释放但新加密 token 已丢弃。 |
| 影响 | 用户重新登录同一 ChatGPT 账号时返回 500，新换发的有效 token 被丢弃（encrypt 后未落库），凭据无法更新；用户被迫手动删除账号才能再次登录（oauth_codex 账号被通用 create 端点拒绝，accounts.rs:95-100）。阻断 re-authorization/re-login 流（refresh 有效时的 silent refresh 路径不受影响，故非阻断凭据刷新总体）。 |
| 修复方向 | 在 `handle_callback` 内对已存在 account_id 走 `accounts::update`（覆盖 credentials_encrypted/name），仅当不存在时再 `accounts::create`；或登录前删除同 account_id/oauth_codex 旧记录后重建。 |
| 验证摘要 | 2 名 verifier 均确认 real，追踪 JWT 解析、id 复用、纯 INSERT、PK 冲突、错误分支丢弃 token、无 update 回退。维持 P1（功能 500 + 手动 workaround，非数据丢失/致命路径）。 |

### P1-4 流式链路测试端到端损坏：api.ts request() 对 SSE 响应强解 JSON

| 项 | 内容 |
|----|------|
| 标题 | 流式链路测试端到端损坏：api.ts `request()` 对 SSE 响应强解 JSON 导致所有 stream=true 测试报错 |
| 文件:行 | `src/lib/api.ts:423` / `src/lib/api.ts:45` |
| 触发 | RoutesPage TestPanel 默认 `stream=true`（`useState(true)`，RoutesPage.tsx:155）。用户点「发送测试」→ `testsApi.run` 调共享 `request<TestResponse>('/tests', {POST})`（api.ts:423-427）。`request()`（api.ts:45-58）通过 `resp.ok`（SSE 返回 200），随后无条件 `return resp.json()`。后端 `tests.rs:94-102` 在 stream=true 透传上游 `text/event-stream` 原始响应体，非 JSON → `resp.json()` 抛 SyntaxError 进入 onError，`setResult({status:0, body:{}, duration_ms:0, endpoint_id:null, error:e.message})`。 |
| 影响 | stream=true 的链路测试永远显示 JSON 解析失败错误而非真实流式输出，`x-test-duration-ms` 头被丢弃（duration_ms=0）。流式测试这一整条功能不可用，违反 PRD「流式：透传上游 text/event-stream 响应体」契约，用户误以为端点/路由坏了。非流式路径正常。 |
| 修复方向 | 流式测试应单独走 EventSource（按 spec）或在 TestsPage 直接用 fetch 读取 ReadableStream 文本逐块显示，而非复用会调 `resp.json()` 的 `testsApi.run`。 |
| 验证摘要 | 2 名 verifier 均确认 real，追踪后端透传、前端 `resp.json()` 无条件执行、无任何 EventSource/ReadableStream/getReader 存在（grep 空）。维持 P1（默认面向用户诊断功能整体损坏，但仅限测试面板范围，不触及代理/数据完整性运行面，故非 P0）。 |
| 去重合并 | 合并 finder-1（api.ts:423，「route wiring」维度）与 finder-2（api.ts:45，「frontend-ui」维度）的同源缺陷，保留更精确的前端触发描述。 |

### P1-5 Dashboard 任何 widget 都无 error 状态：API 失败时静默回退为空数据并误触发首次引导

| 项 | 内容 |
|----|------|
| 标题 | Dashboard 任何 widget 都无 error 状态：API 失败时静默回退为空数据并误触发首次引导 |
| 文件:行 | `src/pages/DashboardPage.tsx:43` |
| 触发 | 后端 42567 不可达 / 某资源 GET 返回 500 / 网络断开时，任一 useQuery 进入 error 态。`main.tsx:11` 设 `retry:1`，TanStack 重试 1 次后 status='error'。`api.ts:50-52` 对非 2xx 抛错、fetch 对网络拒收。`DashboardPage.tsx:43-70` 所有 7 个 useQuery 仅解构 `data: X = []` 与 `isLoading`，从不解构 `error`/`isError`。错误态下 data 回退到 `[]`、isLoading=false。CountCard（285-300）仅判 `loading`，渲染 value=0 无错误指征。EmptyGuide 守卫（90-96, 108）为 `allLoaded && totalResources === 0`，无 `!anyError` 门，故在错误态 allLoaded=true、totalResources=0 → 渲染 EmptyGuide「尚未添加任何账号…请先添加上游账号」+ 「前往账号页」按钮，主动误导运维。 |
| 影响 | 任何 fetch 失败时 Dashboard 静默把所有统计卡显示为 0（看起来用户无资源），并渲染首次运行欢迎卡引导用户去账号页，而非告知后端不可达。掩盖真实错误并误导运维。 |
| 修复方向 | 每个 useQuery 解构 `error`/`isError`，渲染错误态（每卡错误信息或顶部 banner）；EmptyGuide 守卫增加 `!anyError` 与 `allLoaded && totalResources === 0` 并列。 |
| 验证摘要 | 2 名 verifier 均确认 real，追踪 retry:1 配置、无 isError 检查、`= []` fallback、allLoaded/totalResources 在错误态皆满足、无任何防御代码。维持 P1（运维误导的功能性错误，掩盖后端失败，但无崩溃/数据丢失）。 |

---

## 4. P2 发现表

| # | 标题 | 文件:行 | 触发 / 影响 |
|---|------|---------|-------------|
| P2-1 | ChatToResponses 流式在 finish_reason 时发 response.completed 但未先发每个 function_call 的 response.output_item.done | `src-tauri/src/services/translator/openai_responses.rs:362` | 入站 openai-chat、上游 openai-responses且回应含工具调用：`output_item.added(in_progress)` + arguments.delta + finish_reason；缺失 done 使严格客户端不 finalize function_call 的 arguments/状态，工具调用项停在 in_progress。 |
| P2-2 | ChatToResponses / ResponsesToChat 两方向均不映射 max_tokens ↔ max_output_tokens | `src-tauri/src/services/translator/openai_responses.rs:35` | Chat→Responses 上游收到无法识别的 max_tokens、缺 max_output_tokens；Responses→Chat 上游收到无法识别的 max_output_tokens、缺 max_tokens。两方向限制丢失或引发 400。 |
| P2-3 | ChatToAnthropic 剥离 max_completion_tokens 但不映射到 max_tokens（Anthropic 必填 max_tokens） | `src-tauri/src/services/translator/anthropic_openai.rs:740` | 入站 openai-chat 请求仅含 `max_completion_tokens=N` 路由到 Anthropic：被删后 max_tokens 未设 → Anthropic 返回 400 missing max_tokens 或退回默认上限不受约束。 |
| P2-4 | AnthropicToChat input_json_delta 在 block_to_tool_index 缺失时回退 tool_index=0，可能把参数累积到错误工具 | `src-tauri/src/services/translator/anthropic_openai.rs:540` | 上游畸形流在 content_block_start 前先发 input_json_delta，delta 路由到工具 0 与其它工具参数串扰。真实 Anthropic 上游总是先发 content_block_start，低概率。 |
| P2-5 | anthropic_thinking_to_reasoning_effort 读取嵌套 effort 用单点号键，恒回退 "medium" | `src-tauri/src/services/translator/helpers.rs:189` | `Value::get("output_config.effort")` 查找的是字面键名 "output_config.effort" 而非嵌套路径，返回 None 回退 medium；round trip high→medium、low→medium，sampling effort 错误。与反向 writer 配对破坏。 |
| P2-6 | extract_content_text 仅返回首个 text 块；多块 Anthropic system prompt 被静默截断 | `src-tauri/src/services/translator/helpers.rs:48` | AnthropicToChatTranslator::translate_request 对顶层 system field 调用，system 为多 text 块时只回 part1，丢失用户指令/cache anchors。 |
| P2-7 | build_error_event openai-chat 分支静默丢弃错误消息 | `src-tauri/src/services/translator/helpers.rs:92` | openai-chat match arm 忽略 msg，发固定字符串无消息；openai-chat 客户端收到 `finish_reason:"error"` + 空 delta 无法定位失败原因。跨层：错误原因在上游/translator 已生成但永不到达 openai-chat 客户端。 |
| P2-8 | Upstream stream error 未设 errored=true，错误事件每块重复发出且尾部残留 flushed 行 | `src-tauri/src/http/proxy/sse.rs:74` | 上游流发出 `Some(Err(_))` 后 sse.rs:74-80 发 build_error_event 但未置 errored=true；下一轮 poll 仍读内流，可能再发错误帧并 flush 残留行。与 translate_stream_line-Err 分支（65-67 正确置 errored）行为不一致。 |
| P2-9 | 同端点重试无 backoff；SAME_ACCOUNT_RETRY_DELAY_MS(500ms) 定义但从未应用（PRD R3 偏离） | `src-tauri/src/http/proxy/mod.rs:455` | 可重试错误返回后 should_retry=true，立即 continue 重发到同一端点无延迟；constants.rs:32 的 500ms 常量成死代码；PRD R3.3/route policy 指定 retryDelay=500ms。 |
| P2-10 | Stream-guard inline-error ProxyError retryable=false，should_retry 不在 first-chunk 错误上尊重 same_account_retries | `src-tauri/src/http/proxy/stream_guard.rs:116` | 首块为 inline error event 无数值 status 时 建 `UpstreamError(502)` 默认 retryable=false；mod.rs 流式-Err 分支直接 record_failure+continue 换端点，跳过同账号重试。与非流式 429/529/5xx 重试行为不一致。 |
| P2-11 | AuthError（预检刷新失败）冷却 30s 与 PRD「auth 类冷却 5 分钟」契约不符 | `src-tauri/src/http/proxy/failover.rs:182` | OAuth 刷新失败 → AuthError → `calculate_cooldown_seconds` 走 `_ => 30` 默认返回 30s 而非 PRD 第 92 行规定的 300s；30s 后 selector 重选同凭据已失效端点再失败，反复无效重试放大上游错误率。 |
| P2-12 | OAuth 刷新使用 reqwest::Client 无超时，可能无限挂起阻塞故障转移主循环 | `src-tauri/src/http/proxy/oauth_refresh.rs:168` | auth.openai.com 接受 TCP 后迟迟不返回响应体（网络中间盒静默丢包/TLS 半挂）→ `client.post().send().await` 永久等待，inject_auth 阻塞主循环，该请求卡死，无法触发故障转移。 |
| P2-13 | 刷新响应未携带新 refresh_token 时保留旧值，旧 refresh_token 可能已被服务端轮换作废 → 下次刷新失败 + 账号长期失效 | `src-tauri/src/http/proxy/oauth_refresh.rs:206` | 刷新成功但响应无 refresh_token 且 OpenAI 轮换了 refresh_token → 旧 refresh_token 已作废仍写回 DB，下次预检刷新必 400 invalid_grant，账号在重启/下次刷新前持续不可用。 |
| P2-14 | Empty /v1/models data array 静默禁用所有先前同步的模型 | `src-tauri/src/services/model_sync.rs:142` | 上游 GET /v1/models 返回 200+`{"data":[]}`（auth/billing 短暂抖动常见）→ fetch 返回空 Vec → mark_unavailable_except_in_tx 把所有先前同步行 WHERE last_seen_at < sync_time 全部翻为 is_available=0，从 alias 解析候选消失，端点静默不再被选为该模型的 failover 候选，直到后续非空 sync 重启。 |
| P2-15 | Sync upsert 覆盖 custom 命名行但 source 保持 'custom'，逃出 sync 可用性管理 | `src-tauri/src/db/dao/endpoint_models.rs:124` | source='custom' 行存在 (EP,'gpt-4o')，上游 /v1/models 返回同名 synced 模型 → ON CONFLICT DO UPDATE 未包含 source，SQLite 保留 custom，但 display_name/capabilities/context_window/is_available/last_seen_at 被覆盖；该行被 mark_unavailable 过滤（source='synced'）漏掉，永不 prune。源契约数据损坏 + 不一致可用性。 |
| P2-16 | Replace 导入保留包中不存在的 route_settings 残留行，产生混合状态 DB | `src-tauri/src/services/portability/apply.rs:164` | apply_replace 对 route_settings 只 INSERT...ON CONFLICT(id) DO UPDATE，不先 DELETE（accounts/endpoints/models/aliases/tool_takeover 均 DELETE-then-INSERT）。pre-v6 备份导入到含 v1 的目标时残留 v1 行，混合状态违反 replace=全表覆盖契约。**原 P1 降级 P2**：影响范围窄（受 migration-seeded 固定行集合约束，REST API 不允许新建自定义 route row），仅保留而非实际数据丢失。 |
| P2-17 | Replace 导入静默丢弃 ui_settings（app_metadata 偏好键） | `src-tauri/src/services/portability/apply.rs:183` | apply_replace 从不引用 p.ui_settings（仅 apply_merge 写 app_metadata）；full_backup restore 不还原被 collect 捕获的 auto_model_refresh_enabled 等偏好，用户偏好静默回退默认，round-trip 保真中断。**原 P1 降级 P2**：唯一当前白名单键为可手动重切的 UI boolean，无凭据/锁/日志损害，非实际数据丢失。 |
| P2-18 | LogsPage「生产」日志过滤不生效：显示包含 test 日志 | `src/pages/LogsPage.tsx:31` | 选「生产」+工具留空 → effectiveTool=undefined → GET /api/logs 不带 tool → DAO tool=None 不过滤 → 返回含 tool='test' 的测试日志（logger.rs:97-98）；「测试」正向筛选已实现但缺「生产」负向排除。**原 P1 降级 P2**：仅影响管理界面日志筛选语义，无数据/崩溃/代理转发影响，前端 filter 或后端 tool_neq 可低成本修复。 |
| P2-19 | package.mode 与 package.kdf 独立校验，允许 crafted/编辑包跨绑密钥源（master key vs password） | `src-tauri/src/services/portability/mod.rs:135` | 编辑 full_backup 包 JSON 把 mode 改 portable 而 kdf 留 argon2id 可触发用用户密码解密 + 破坏性 Replace 策略（DELETE FROM accounts/endpoints）；mode/kdf 从不交叉校验。defense-in-depth 缺口，代码注释声称 full_backup=master key 但未强制。 |
| P2-20 | app_data_dir 回退 '.' 在 HOME/USERPROFILE 均缺时用 CWD，非可移植 DB 位置 + CWD 变更可能数据丢失 | `src-tauri/src/config/paths.rs:17` | dirs::data_dir() 返回 None 且 HOME/USERPROFILE 都缺（headless/service/embedded 账号清空 env）→ `dirs_or_fallback()` 返 `PathBuf::from('.')`，DB 落在 ./agent-switch；后续不同 CWD 启动数据消失（看似空 app）。 |
| P2-21 | start_codex_login 对任何 start_login 失败都返回 409 CONFLICT，而非仅 session-in-progress | `src-tauri/src/http/api/auth.rs:39` | TcpListener bind 1455 失败/OAuth metadata fetch 失败/config 错误均映射 CONFLICT；前端无法区分「已有登录进行中」与其它内部失败，误导错误 UX。 |
| P2-22 | OAuth 回调 state 不匹配时不清理 session/回调服务，1455 端口与 session 永久占用 | `src-tauri/src/services/codex_oauth.rs:207` | 回调 state 与 expected_state 不一致（CSRF 噪声/多 tab/第三方重定向到 localhost:1455）→ handle_callback 直接 return 未 cleanup_session；此后任何 start_login 都返回「已有登录进行中」，用户无法再次登录，需重启应用。功能性卡死+资源泄漏。 |
| P2-23 | 初始登录不写入 expires_at（丢弃 expires_in），首次代理请求必触发一次刷新 | `src-tauri/src/services/codex_oauth.rs:243` | TokenResponse.expires_in 被 `#[allow(dead_code)]` 忽略，CodexCredentials.expires_at 硬编码 None；oauth_refresh::ensure_valid_token 见 None 即 needs_refresh=true，首次代理强制刷新往返。跨层契约不一致。 |
| P2-24 | account_id 缺失时回退随机 UUID，导致同一逻辑账号重复登录产生多条账号记录 | `src-tauri/src/services/codex_oauth.rs:265` | id_token 未含 chatgptAccountId/accountID 时 uuid::Uuid::new_v4() 生成新 id，每次登录不同；孤立重复账号+凭据冗余，旧失效 token 不清理。 |

> 注：P2-22/P2-23/P2-24 与 P1-3 同属 codex_oauth.rs 凭据链路缺陷族，但触发条件与影响面不同，保留为独立条目。

---

## 5. P3 代码质量附录

> 不进主排序，按子系统聚类列举。

**Translator 层**
- `openai_responses.rs:680` — ResponsesToChat 每遇 `event:` 行重置 content_block_index=0（死代码/误导，与 AnthropicToChat 注释矛盾）。
- `anthropic_openai.rs:547` — AnthropicToChat input_json_delta 累积的 acc.arguments 永不被输出（死状态，易误导维护者忽略 P1-2 的 content_block_stop 缺失）。
- `helpers.rs:16` — map_role to_anthropic docstring 承诺 tool→user 但代码 tool→tool，且全仓零生产 call-site（dead code + 文档矛盾）。
- `helpers.rs:59` — extract_all_text / extract_delta_text / is_sse_event_end 三个导出 helper 在生产中无调用（translators 内联重实现）。
- `helpers.rs:85` — build_error_event anthropic fallback 的 unwrap_or_else 分支不可达，若运行对含引号/反斜杠消息会产生非法 JSON。

**Proxy/Stream/Failover 层**
- `stream_guard.rs:94` — 首块为空时设 stream_started=false 仍返回 Ok(response)，客户端得到无终止帧的空 SSE 流可能挂起。
- `sse.rs:561` — translate_stream 跨协议 fallback resolve Passthrough（from==from）静默把上游错误协议 SSE 透传给中流客户端（当前全覆盖的 Pair 注册使其不可达）。
- `mod.rs:445` — 所有上游响应头被丢弃，客户端丢失 x-request-id、anthropic-ratelimit-*、openai-organization 等。
- `stream_guard.rs:148` — extract_error_code 忽略 Retry-After / overload type 在 inline stream errors，rate-limited 流错误丢失 retry 提示。
- `mod.rs:570` — 跨协议流式丢弃上游 keep-alive (ping) 事件，长上游暂停可能让客户端 idle-timeout。
- `failover.rs:142` — route_settings.cooldown_multiplier DB/API/UI 可调但从不应用（死配置）。
- `oauth_refresh.rs:22` — REFRESH_LOCKS 静态 HashMap 无清理，每个 account_id 永久持有一个 Mutex（有界，影响轻微）。
- `codex_oauth.rs:244` — 首次 OAuth 登录落库 credentials.expires_at 恒 None → 每个新账号首次请求多触发一次刷新。
- `failover.rs:367` — mod.rs 上游非成功响应 `.bytes().await.unwrap_or_default()` 吞错误响应体读取失败（极低概率，轻微）。
- `codex_oauth.rs:363` — exchange_code_for_token / fetch_models_from_endpoint 用 `Client::new()` 无超时，上游卡死时回调与顺序同步 do_sync_all 永久挂起。
- `codex_oauth.rs:398` — parse_jwt_fields 用 URL_SAFE_NO_PAD 对含 '=' padding 的 JWT payload 解码失败，账号名降级 'Codex 账号'。

**Portability 层**
- `portability/mod.rs:124` — format_version 不等硬拒无迁移路径，未来格式 bump 会使所有现有 .asbak/.ascfg 失效。
- `crypto_box.rs:46` — weak_password_warning 用字节长度非字符数，CJK 密码阈值错误（仅 warning 不强制）。
- `portability/mod.rs:281` — backup_db_file 用 Iso8601::DEFAULT 时间戳仅替换 ':'，'+'/可能含其它标点使 Windows 文件名不确定。
- `apply.rs:308` — apply merge 对孤儿 endpoint_model/alias（endpoint 不在包内）静默 `continue` 跳过，无 ImportReport 警告；replace 在 FK 关闭时可能插入悬挂行。

**model_sync / tool_takeover**
- `model_sync.rs:87` — do_sync_all 的 host_last HashMap 仅 insert 从不读（死代码，注释声称按 host 分组限流未实现）。
- `model_sync.rs:237` — fetch_models_from_endpoint 对所有模型硬编码 capabilities=["chat","streaming","tool_calling"]，list_capable/has_capable 误报。
- `tool_takeover/mod.rs:132` — enable 流程 apply 成功后 upsert_state 失败时配置已改但 DB 记 enabled=false（状态不一致卡接管态）。
- `tool_takeover/mod.rs:298` — atomic_write 临时文件 with_extension('tmp') 丢原扩展名，残留 settings.tmp/config.tmp。

**Route / Handler / Config**
- `http/api/models.rs:131` — delete_one 用 `let _ =` 丢弃 mark_alias_invalid_for_model 结果，隐藏 DB 错误无事务耦合。
- `http/api/v1_models.rs:102` — v1_models 返回重复模型名的非确定性 owned_by（endpoint_models::list 无 ORDER BY）。
- `http/api/tools.rs:33` — ToolStatusResponse live_category 经 to_value/from_value 圆旅 + 'unrecognized' fallback 是脆弱死塑造层。
- `http/api/models.rs:107` — create_custom 用 unwrap_or_default() 可能对 Some(vec) 存 '' 行（Vec<String> 实际从不出错，防御死代码）。
- `http/api/models.rs:95` — sync_all 对任何 model_sync 错误返回 409 CONFLICT（DNS/上游 4xx 也映射 conflict）。
- `http/api/routes.rs:137` — update 在事务外 get-then-upsert，固定 route id 上并发 PUT 可能交织丢失更新（窗口极小）。

**Database**
- `db/dao/request_logs.rs:229` — prune_old 子查询同表 id 反连接，O(N²) 成本与非确定性 tie-break。
- `db/dao/endpoint_models.rs:198` — capability LIKE '%cap%' 子串匹配无 token 边界，未来能力前缀冲突会误过滤。

**Frontend**
- `src/components/layout/PagePlaceholder.tsx:1` — PagePlaceholder 组件死代码，无任何引用（all 8 pages 已实现）。
- `src/lib/utils.ts:3` — cn 工具函数死代码（5 行模块零消费者）。
- `DashboardPage.tsx:17` — TOOL_LABELS/CATEGORY_LABELS/CATEGORY_COLORS 在 DashboardPage 与 ToolCard 完全重复（维护隐患，与跨层指南 Mistake 4 反模式）。
- `DashboardPage.tsx:595` — formatTime 在 DashboardPage 与 LogsPage 逐字重复。
- `DashboardPage.tsx:53` — queryFn 包裹不一致（modelsApi.list 箭头包裹而其它直接传引用，纯风格）。
- `DashboardPage.tsx:64` — logs queryKey 形态在 Dashboard（['logs']）与 LogsPage（['logs', params]）不一致（非当前碰撞）。
- `RoutesPage.tsx:386` — 「连续点同一测试按钮」原 P2 finder 描述含乱码/语义不明，且与 P1-4 同处测试路径；保留为低置信 P3 级观察项，不作为独立修复项。

---

## 6. 被证伪/降级的原 P0/P1

保留可追溯性。survives=false 或经 verifier 改判的原 P0/P1 条目：

| 原条目 | 原严重度 | 文件:行 | 处理 | verifier 证伪/降级理由 |
|--------|----------|---------|------|------------------------|
| ChatToAnthropic 流式不发出 text 的 content_block_start/stop，且 tool_use index 与 text index 0 冲突 | P1 | `anthropic_openai.rs:943` | 证伪 | survives=false。verifier 重新追踪后判定：(a) text 块无 content_block_start、(b) index 0 冲突两大子项不成立/不可独立存活——其 wire-format 主张被拆分后仅 `content_block_stop` 缺失子项（968 行）作为独立 P1 存活（见 P1-2）。原 943 条整体被标 survives=false、不作为独立 P1 入表。 |
| Replace import leaves stale route_settings rows | P1 | `apply.rs:164` | 降级 P1→P2 | 2 verifier 均判 real 但 corrected_severity=P2：影响范围窄（route_settings id 集合小且受 migration-seeded 固定行约束，REST API 不允许新建自定义 route row，upsert DAO 注释「尚未接线」），是「保留否则有效行」而非广泛数据丢失，无崩溃/凭据损害。维持 P2 见 P2-16。 |
| Replace import silently drops ui_settings | P1 | `apply.rs:183` | 降级 P1→P2 | 2 verifier 均判 real 但 corrected_severity=P2：当前唯一白名单键为 trivially 可重切的 UI boolean auto_model_refresh_enabled，无凭据/锁/日志损害，restore-fidelity/preference-loss 而非 actual data loss。维持 P2 见 P2-17。 |
| LogsPage 生产日志过滤不生效 | P1 | `LogsPage.tsx:31` | 降级 P1→P2 | 2 verifier 均判 real 但其中 1 名 corrected_severity=P2：仅影响管理界面日志筛选语义正确性，无数据损坏/崩溃/代理转发/安全影响，仅增加运维噪声，存在前端 filter 或后端 tool_neq 的低成本修复路径。维持 P2 见 P2-18。 |

> 验证后实际存活 P0/P1：5 条（P1-1 ~ P1-5）；被证伪：1 条（943）；降级 P1→P2：3 条（route_settings/ui_settings/生产日志）。另 1 条跨 finder 重复（api.ts:45 并入 api.ts:423 形成去重合并 P1-4），不计入 dropped_or_downgraded。

---

## 7. 已知限制（非本轮新发现，单独成节）

以下为本轮审计之外、codebase 已知状态，列出以划定边界，不计入发现计数：

- **`cargo fmt --check` 当前失败**：约 10 处格式漂移，集中在 translator/db/proxy 的近期修复 commit（见近期 `fix(translator)` / `fix(db,proxy)` / `fix(ui)` 系列 commit），需后续 `cargo fmt` 收敛。
- **Dashboard 总览页部分为占位**：Session 7 journal 注，部分统计/可视化尚未落地。
- **跨协议翻译未完全接线**：journal 记录，部分协议对尚未在 proxy 层路由接通。
- **role_mapping 为简化 stub**：journal 记录，未实现完整角色映射（与本轮 helpers.rs:16 dead-code map_role 关联）。
- **spec 层错位**：`.trellis/spec/backend|frontend` 的 index 描述 Trellis 工具层而非 agent-switch 应用本体；仅 `.trellis/spec/guides/app-stack-conventions.md` 是应用相关内容。

---

## 8. 覆盖矩阵

| 维度 | 覆盖说明 | 发现数 | 验证结果 |
|------|----------|--------|----------|
| SSE streaming assembly & index mapping (anthropic_openai.rs, openai_responses.rs) | anthropic_openai.rs 全读（1667 LOC 含 mod tests）；openai_responses.rs 全读（1186 LOC 含 mod tests）；旁读 mod.rs/helpers.rs/sse.rs/translate.rs 验跨层契约。无采样。 | 8 | 1 P1 证伪(943)→降级；1 P1 存活(968, P1-2)；其余 P2/P3 存活 |
| translator non-streaming + shared helpers (helpers.rs, mod.rs, native.rs) | 三个目标文件全读（native.rs 93 / helpers.rs 337 / mod.rs 269 LOC）；旁读 anthropic_openai.rs 全读、openai_responses.rs 全读、sse.rs 40-129 行采样确认 build_error_event 契约；grep 验证每 个 exported helper 生产用法。 | 6 | 全部存活（5 P2 + 3 P3 并入本维 + 跨维） |
| proxy-forwarding-stream-guard-capability-sse | mod.rs(823)/stream_guard.rs(211)/capability.rs(133)/translate.rs(144)/sse.rs(198) 全读；旁读 failover/error/constants/translator 全文验跨层契约。无采样或未读。 | 9 | 1 P1 存活(P1-1)；其余 P2/P3 存活 |
| proxy-failover-oauth | failover.rs(236)/oauth_refresh.rs(234) 全读；旁读 error/constants/mod.rs/selector/auth_injector/codex_oauth/crypto/dao/route_settings/api routes/migrations(grep)/spec/guides。未采样跳读。 | 7 | 全部存活（1 P2-11~13 + P3 + 跨维） |
| database-layer (migrations, dao, tx, locks, pruning) | 5 个目标文件全读（migrations.rs 332/endpoints.rs 258/endpoint_models.rs 221/accounts.rs 190/request_logs.rs 277）；旁读 connection.rs/model_sync.rs 全文验 tx 含入与 Mutex 跨 await；确认 migration 各表各事务、参数化、busy_timeout=WAL+FK=ON、sync 不持锁跨 await、seed 用 ISO8601 'Z' 跨层一致。 | 4 | 全部存活（2 P2[P2-14,P2-15] + 2 P3） |
| codex_oauth + model_sync + tool_takeover | 三个范围文件全读（codex_oauth.rs 444/model_sync.rs 266/tool_takeover/mod.rs 310）；旁读 dao/crypto/tool_takeover 子模块/oauth_refresh/auth.rs/accounts.rs 80-117/migrations.rs 26-125。无采样。 | 11 | 1 P1 存活(P1-3)；其余 P2/P3 存活 |
| portability (import/export JSON versioning + crypto_box AEAD + apply idempotency + collect/apply round-trip + credential handling) | 四个目标文件全读（mod.rs 479 含 180 LOC test/apply.rs 444/collect.rs 246/crypto_box.rs 184）；旁读 package.rs/crypto.rs b64 段/migrations 1-5 schema/keychain load_master_key。无跳读。 | 7 | 2 原 P1 降级 P2(P2-16,P2-17)；其余 P2/P3 存活 |
| route wiring & handler error mapping & AppState mutation safety & config/paths | 15 个目标文件全读（含 2 spec + 10 api/routes handlers + app/mod commands + app_state + 2 config）；旁读 router/mod/error/placeholders/api mod/api tests/lib/api.ts 全读；Grep-only 采样 5 个支撑源印证契约。无目标文件未读剩余。 | 9 | 1 P1 存活(P1-4 共享)；其余 P2/P3 存活 |
| frontend-ui-mutation-guards-clamping-import-reset-errors | 14 个目标文件全读（api.ts 428 + 8 pages + 5 components + 2 spec）；跨层对照后端 logs.rs/aliases.rs/tests.rs/grep 校验。无目录或文件未读。 | 3 | 1 P1 存活(P1-4 合并)；1 P1 降级 P2(P2-18)；1 跨维 P2→P3 观察(RoutesPage:386) |
| frontend-dashboard-states-and-duplication | 6 个目标文件全读（DashboardPage 605/App 28/main 26/AppShell 61/PagePlaceholder 15/utils 5）；旁读 api.ts/LogsPage/ToolCard/RoutesPage 1-40 + 后端 routes.rs 验跨层 payload。无采样。 | 7 | 1 P1 存活(P1-5)；其余 P3 全存活 |

**覆盖总计**：10 维 / 71 finder 原始计数（含 1 条跨维重复 api.ts:45↔423）。去重 + 证伪后 **69 条存活**：5 P1 + 24 P2（含 3 条由原 P1 降级）+ 40 P3（含 1 条由原 P2 观察项）；4 条原 P0/P1 被证伪/降级（见 §6）。

---

## 9. 退化/采样声明

本轮 finder 阶段已声明各目标文件全读，但以下为实际采样/旁读区段（非目标文件本体），用于跨层契约验证而非主发现定位：

- **sse.rs 40-129 行（采样）** — translator helpers 维度为确认 `build_error_event` call-site 契约采样本区段；其余 sse.rs 由 proxy 维度全读。
- **RoutePage.tsx 1-40 行 / ToolCard 4-18 152-156（旁读）** — frontend-dashboard 维度验证 `['routes']` query-key 共享与重复 map 时旁读起始段。
- **migrations.rs（Grep-only）、route_settings.rs（Grep-only, upsert_partial 签名）、request_logs.rs（Grep, limit/offset guard）、endpoint_models.rs（Grep, capabilities+has_capable_model）、endpoints.rs（Grep, list_by_protocol）** — route wiring 维度的契约印证采样，全部非目标文件、不承担主发现锚点。
- **RoutesPage.tsx 测试面板 140-260 行** — route wiring 维度为验 TestPanel 契约旁读；frontend 维度对该文件全读覆盖重叠区段，无独立未读区段。

无目标文件留下未读剩余；无发现仅基于采样行定位。所有 P0/P1 锚点均来自全读文件。verifier 阶段对每条 P0/P1 进行独立二次追踪，2 名 verifier 结论一致方记 survives=true。

---

*报告生成于 2026-07-03，基于 10 个子系统 finder 输出 + 对抗验证结果。finder 原始 71 条，去重 + 证伪后 69 条存活：确认存活 P1 5 条、P2 24 条、P3 40 条；原 P0/P1 被证伪 1 条、降级 P1→P2 共 3 条。*
