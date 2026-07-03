# 前端状态管理规范

## TanStack Query

- queryKey 必须包含影响结果的参数对象，例如 `['logs', params]`。
- Dashboard 的摘要查询也要带参数 key，避免与详情页列表 queryKey 碰撞。

## Loading / Error / Empty

- loading、error、empty 三种状态必须区分。
- 任一关键 query error 时，不得显示首次使用 EmptyGuide。
- CountCard 等统计卡不能在 error 时静默显示 0；必须显示错误态或顶部错误提示。

## Mutation guard

- 触发真实上游请求或测试请求的按钮必须在 pending 时禁用/guard，避免重复点击产生并发请求。
