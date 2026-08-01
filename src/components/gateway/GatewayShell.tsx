import { useCallback, useEffect, useMemo, useState } from "react";
import {
  Activity,
  ArrowDown,
  ArrowUp,
  BookOpen,
  CircleAlert,
  Copy,
  Database,
  Gauge,
  KeyRound,
  ListTree,
  Play,
  RefreshCw,
  Save,
  Server,
  Settings2,
  ShieldCheck,
  Square,
  Waypoints,
} from "lucide-react";
import { toast } from "sonner";
import { gatewayApi } from "@/lib/api/gateway";
import { proxyApi } from "@/lib/api/proxy";
import type {
  GatewayDomainConfig,
  GatewayMigrationIssue,
  GatewayModel,
  GatewayModelAlias,
  GatewayRoute,
  GatewayRouteHealth,
  GatewayUpstream,
  GatewayUpstreamModel,
} from "@/types/gateway";
import type { GatewayAuthStatus, ProxyStatus } from "@/types/proxy";
import { Alert, AlertDescription, AlertTitle } from "@/components/ui/alert";
import { Badge } from "@/components/ui/badge";
import { Button } from "@/components/ui/button";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/card";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { Switch } from "@/components/ui/switch";

const NAV_ITEMS = [
  "Overview",
  "上游",
  "模型",
  "路由",
  "日志统计",
  "设置",
  "接入文档",
] as const;

type GatewayNavItem = (typeof NAV_ITEMS)[number];

const EMPTY_STATUS: ProxyStatus = {
  running: false,
  address: "127.0.0.1",
  port: 42567,
  active_connections: 0,
  total_requests: 0,
  success_requests: 0,
  failed_requests: 0,
  success_rate: 0,
  uptime_seconds: 0,
  last_request_at: null,
  last_error: null,
  failover_count: 0,
};

const EMPTY_AUTH: GatewayAuthStatus = {
  authRequired: true,
  keys: [],
};

interface GatewayData {
  config: GatewayDomainConfig | null;
  upstreams: GatewayUpstream[];
  upstreamModels: GatewayUpstreamModel[];
  models: GatewayModel[];
  aliases: GatewayModelAlias[];
  routes: GatewayRoute[];
  routeHealth: GatewayRouteHealth[];
  migrationIssues: GatewayMigrationIssue[];
}

const EMPTY_DATA: GatewayData = {
  config: null,
  upstreams: [],
  upstreamModels: [],
  models: [],
  aliases: [],
  routes: [],
  routeHealth: [],
  migrationIssues: [],
};

export function GatewayShell() {
  const [activeNav, setActiveNav] = useState<GatewayNavItem>("Overview");
  const [data, setData] = useState<GatewayData>(EMPTY_DATA);
  const [status, setStatus] = useState<ProxyStatus>(EMPTY_STATUS);
  const [auth, setAuth] = useState<GatewayAuthStatus>(EMPTY_AUTH);
  const [busy, setBusy] = useState(false);
  const [keyName, setKeyName] = useState("本机客户端");
  const [createdSecret, setCreatedSecret] = useState<string | null>(null);

  const refresh = useCallback(async () => {
    const [
      config,
      upstreams,
      upstreamModels,
      models,
      aliases,
      routes,
      migrationIssues,
      routeHealth,
      nextStatus,
      nextAuth,
    ] = await Promise.all([
      gatewayApi.getDomainConfig(),
      gatewayApi.listUpstreams(),
      gatewayApi.listUpstreamModels(),
      gatewayApi.listModels(),
      gatewayApi.listModelAliases(),
      gatewayApi.listRoutes(),
      gatewayApi.listMigrationIssues(),
      gatewayApi.listRouteHealth(),
      proxyApi.getProxyStatus(),
      proxyApi.getGatewayAuthStatus(),
    ]);

    setData({
      config,
      upstreams,
      upstreamModels,
      models,
      aliases,
      routes,
      routeHealth,
      migrationIssues,
    });
    setStatus(nextStatus);
    setAuth(nextAuth);
  }, []);

  useEffect(() => {
    refresh().catch((error) => toast.error(String(error)));
  }, [refresh]);

  const run = async (action: () => Promise<unknown>, message: string) => {
    setBusy(true);
    try {
      await action();
      await refresh();
      toast.success(message);
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const copy = async (value: string, message = "已复制") => {
    try {
      await navigator.clipboard.writeText(value);
      toast.success(message);
    } catch (error) {
      toast.error(`复制失败：${String(error)}`);
    }
  };

  const createKey = async () => {
    const name = keyName.trim();
    if (!name) {
      toast.error("请输入 API Key 名称");
      return;
    }

    setBusy(true);
    try {
      const created = await proxyApi.createGatewayApiKey(name);
      setCreatedSecret(created.secret);
      await refresh();
      toast.success("API Key 已创建，请立即复制保存");
    } catch (error) {
      toast.error(String(error));
    } finally {
      setBusy(false);
    }
  };

  const gatewayUrl = useMemo(
    () =>
      `http://${data.config?.listenAddress ?? "127.0.0.1"}:${data.config?.listenPort ?? 42567}`,
    [data.config],
  );

  return (
    <div className="min-h-screen bg-background p-6 text-foreground">
      <div className="mx-auto flex max-w-7xl flex-col gap-5">
        <header className="flex flex-wrap items-center justify-between gap-4">
          <div>
            <p className="text-sm font-medium text-blue-500">Agent Switch</p>
            <h1 className="text-2xl font-semibold">本地模型网关</h1>
            <p className="mt-1 text-sm text-muted-foreground">
              只管理 Agent Switch DB，不探测、不读取或修改任何客户端配置。
            </p>
          </div>
          <Button variant="outline" onClick={() => refresh()} disabled={busy}>
            <RefreshCw className="mr-2 h-4 w-4" />
            刷新
          </Button>
        </header>

        <nav className="flex gap-1 overflow-x-auto rounded-xl border bg-card p-1">
          {NAV_ITEMS.map((item) => (
            <Button
              key={item}
              variant={activeNav === item ? "secondary" : "ghost"}
              className="shrink-0"
              onClick={() => setActiveNav(item)}
            >
              {item}
            </Button>
          ))}
        </nav>

        {activeNav === "Overview" && (
          <OverviewPage
            data={data}
            status={status}
            auth={auth}
            busy={busy}
            onStart={() => run(() => proxyApi.startProxyServer(), "网关已启动")}
            onStop={() => run(() => proxyApi.stopProxyServer(), "网关已停止")}
          />
        )}
        {activeNav === "上游" && <UpstreamsPage data={data} />}
        {activeNav === "模型" && (
          <ModelsPage
            data={data}
            busy={busy}
            onToggle={(model) =>
              run(
                () => gatewayApi.setModelEnabled(model.id, !model.enabled),
                model.enabled ? "模型已停用" : "模型已启用",
              )
            }
            onActivate={(model) =>
              run(
                () => gatewayApi.setModelState(model.id, true, "active"),
                "模型已确认并启用",
              )
            }
          />
        )}
        {activeNav === "路由" && (
          <RoutesPage
            data={data}
            busy={busy}
            onToggle={(route) =>
              run(
                () => gatewayApi.setRouteEnabled(route.id, !route.enabled),
                route.enabled ? "路由候选已停用" : "路由候选已启用",
              )
            }
            onReorder={(gatewayModelId, orderedIds) =>
              run(
                () => gatewayApi.reorderRoutes(gatewayModelId, orderedIds),
                "路由顺序已更新",
              )
            }
          />
        )}
        {activeNav === "日志统计" && (
          <ObservabilityPage data={data} status={status} />
        )}
        {activeNav === "设置" && (
          <SettingsPage
            config={data.config}
            status={status}
            auth={auth}
            busy={busy}
            keyName={keyName}
            createdSecret={createdSecret}
            onConfigChange={(config) =>
              setData((current) => ({ ...current, config }))
            }
            onSave={(config) =>
              run(
                () =>
                  gatewayApi.updateDomainConfig({
                    ...config,
                    listenAddress: "127.0.0.1",
                  }),
                "网关设置已保存",
              )
            }
            onKeyNameChange={setKeyName}
            onCreateKey={createKey}
            onRevokeKey={(keyId) =>
              run(() => proxyApi.revokeGatewayApiKey(keyId), "API Key 已撤销")
            }
            onCopy={copy}
          />
        )}
        {activeNav === "接入文档" && (
          <AccessDocsPage gatewayUrl={gatewayUrl} onCopy={copy} />
        )}
      </div>
    </div>
  );
}

function OverviewPage({
  data,
  status,
  auth,
  busy,
  onStart,
  onStop,
}: {
  data: GatewayData;
  status: ProxyStatus;
  auth: GatewayAuthStatus;
  busy: boolean;
  onStart: () => void;
  onStop: () => void;
}) {
  const activeModels = data.models.filter(
    (model) => model.enabled && model.migrationStatus === "active",
  ).length;
  const activeRoutes = data.routes.filter((route) => route.enabled).length;
  const activeKeys = auth.keys.filter((key) => !key.revokedAt).length;

  return (
    <div className="space-y-5">
      <section className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <StatusCard
          icon={<Activity className="h-5 w-5" />}
          title="运行状态"
          value={status.running ? "运行中" : "已停止"}
          detail={`${status.address}:${status.port}`}
        />
        <StatusCard
          icon={<Server className="h-5 w-5" />}
          title="可用上游"
          value={`${data.upstreams.filter((upstream) => upstream.enabled).length}`}
          detail={`共 ${data.upstreams.length} 个上游`}
        />
        <StatusCard
          icon={<Database className="h-5 w-5" />}
          title="已激活模型"
          value={`${activeModels}`}
          detail={`共 ${data.models.length} 个模型`}
        />
        <StatusCard
          icon={<Waypoints className="h-5 w-5" />}
          title="已启用路由"
          value={`${activeRoutes}`}
          detail={`共 ${data.routes.length} 个候选`}
        />
      </section>

      <section className="grid gap-5 lg:grid-cols-[1.2fr_1fr]">
        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <Gauge className="h-5 w-5" />
              网关运行
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-4">
            <div className="grid gap-3 sm:grid-cols-3">
              <Metric label="总请求" value={status.total_requests} />
              <Metric label="成功" value={status.success_requests} />
              <Metric label="失败" value={status.failed_requests} />
            </div>
            <div className="flex flex-wrap gap-2">
              <Button disabled={busy || status.running} onClick={onStart}>
                <Play className="mr-2 h-4 w-4" />
                启动网关
              </Button>
              <Button
                variant="destructive"
                disabled={busy || !status.running}
                onClick={onStop}
              >
                <Square className="mr-2 h-4 w-4" />
                停止网关
              </Button>
            </div>
            <p className="text-xs text-muted-foreground">
              网关启停只影响 Agent Switch 自身服务，不会接管或改写客户端。
            </p>
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle className="flex items-center gap-2">
              <ShieldCheck className="h-5 w-5" />
              安全边界
            </CardTitle>
          </CardHeader>
          <CardContent className="space-y-3 text-sm">
            <SummaryLine label="监听范围" value="仅 127.0.0.1" />
            <SummaryLine
              label="Bearer 鉴权"
              value={auth.authRequired ? "已启用" : "已关闭"}
            />
            <SummaryLine label="有效 API Key" value={`${activeKeys} 个`} />
            <SummaryLine label="客户端配置探测" value="永久关闭" />
          </CardContent>
        </Card>
      </section>

      {data.migrationIssues.length > 0 && (
        <Alert variant="destructive">
          <CircleAlert className="h-4 w-4" />
          <AlertTitle>存在待确认的数据项</AlertTitle>
          <AlertDescription>
            当前有 {data.migrationIssues.length}{" "}
            个迁移问题。冲突或草稿模型不会参与路由，请在“模型”页确认后激活。
          </AlertDescription>
        </Alert>
      )}
    </div>
  );
}

function UpstreamsPage({ data }: { data: GatewayData }) {
  const modelCountByUpstream = useMemo(() => {
    const counts = new Map<string, number>();
    for (const model of data.upstreamModels) {
      counts.set(model.upstreamId, (counts.get(model.upstreamId) ?? 0) + 1);
    }
    return counts;
  }, [data.upstreamModels]);

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Server className="h-5 w-5" />
          上游 Provider
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">
          此处只展示 Agent Switch DB 中的上游，不会从任何客户端配置发现或导入
          Provider。
        </p>
        {data.upstreams.length === 0 ? (
          <EmptyState text="暂无上游数据" />
        ) : (
          data.upstreams.map((upstream) => (
            <div
              key={upstream.id}
              className="grid gap-3 rounded-lg border p-4 md:grid-cols-[1fr_auto] md:items-center"
            >
              <div className="min-w-0">
                <div className="flex flex-wrap items-center gap-2">
                  <p className="font-medium">{upstream.name}</p>
                  <Badge variant={upstream.enabled ? "secondary" : "outline"}>
                    {upstream.enabled ? "启用" : "停用"}
                  </Badge>
                  <Badge variant="outline">
                    {protocolLabel(upstream.protocol)}
                  </Badge>
                </div>
                <p className="mt-1 truncate font-mono text-xs text-muted-foreground">
                  {upstream.baseUrl ?? "未设置上游地址"}
                </p>
                {upstream.notes && (
                  <p className="mt-1 text-xs text-muted-foreground">
                    {upstream.notes}
                  </p>
                )}
              </div>
              <div className="text-left md:text-right">
                <p className="text-lg font-semibold">
                  {modelCountByUpstream.get(upstream.id) ?? 0}
                </p>
                <p className="text-xs text-muted-foreground">上游模型</p>
              </div>
            </div>
          ))
        )}
      </CardContent>
    </Card>
  );
}

function ModelsPage({
  data,
  busy,
  onToggle,
  onActivate,
}: {
  data: GatewayData;
  busy: boolean;
  onToggle: (model: GatewayModel) => void;
  onActivate: (model: GatewayModel) => void;
}) {
  const aliasesByModel = useMemo(() => {
    const aliases = new Map<string, string[]>();
    for (const item of data.aliases) {
      aliases.set(item.gatewayModelId, [
        ...(aliases.get(item.gatewayModelId) ?? []),
        item.alias,
      ]);
    }
    return aliases;
  }, [data.aliases]);

  const routeCountByModel = useMemo(() => {
    const counts = new Map<string, number>();
    for (const route of data.routes) {
      counts.set(
        route.gatewayModelId,
        (counts.get(route.gatewayModelId) ?? 0) + 1,
      );
    }
    return counts;
  }, [data.routes]);

  return (
    <Card>
      <CardHeader>
        <CardTitle className="flex items-center gap-2">
          <Database className="h-5 w-5" />
          网关模型
        </CardTitle>
      </CardHeader>
      <CardContent className="space-y-3">
        <p className="text-sm text-muted-foreground">
          只有 migrationStatus=active 且已启用的模型才会参与精确模型路由。
        </p>
        {data.models.length === 0 ? (
          <EmptyState text="暂无网关模型" />
        ) : (
          data.models.map((model) => {
            const isActive = model.migrationStatus === "active";
            return (
              <div
                key={model.id}
                className="flex flex-wrap items-center justify-between gap-4 rounded-lg border p-4"
              >
                <div className="min-w-0 space-y-1">
                  <div className="flex flex-wrap items-center gap-2">
                    <p className="font-medium">
                      {model.displayName || model.modelId}
                    </p>
                    <Badge variant={isActive ? "secondary" : "destructive"}>
                      {isActive ? "已激活" : model.migrationStatus}
                    </Badge>
                    <Badge variant="outline">{model.source}</Badge>
                  </div>
                  <p className="font-mono text-xs text-muted-foreground">
                    {model.modelId}
                  </p>
                  <p className="text-xs text-muted-foreground">
                    别名：
                    {(aliasesByModel.get(model.id) ?? []).join("、") || "无"}
                    {" · "}路由候选：{routeCountByModel.get(model.id) ?? 0}
                  </p>
                </div>
                {isActive ? (
                  <div className="flex items-center gap-2">
                    <Label htmlFor={`model-${model.id}`}>
                      {model.enabled ? "已启用" : "已停用"}
                    </Label>
                    <Switch
                      id={`model-${model.id}`}
                      checked={model.enabled}
                      disabled={busy}
                      onCheckedChange={() => onToggle(model)}
                    />
                  </div>
                ) : (
                  <Button
                    size="sm"
                    disabled={busy}
                    onClick={() => onActivate(model)}
                  >
                    确认并启用
                  </Button>
                )}
              </div>
            );
          })
        )}
      </CardContent>
    </Card>
  );
}

function RoutesPage({
  data,
  busy,
  onToggle,
  onReorder,
}: {
  data: GatewayData;
  busy: boolean;
  onToggle: (route: GatewayRoute) => void;
  onReorder: (gatewayModelId: string, orderedIds: string[]) => void;
}) {
  const upstreamById = useMemo(
    () => new Map(data.upstreams.map((upstream) => [upstream.id, upstream])),
    [data.upstreams],
  );
  const modelById = useMemo(
    () => new Map(data.models.map((model) => [model.id, model])),
    [data.models],
  );
  const healthByRoute = useMemo(
    () => new Map(data.routeHealth.map((item) => [item.routeTargetId, item])),
    [data.routeHealth],
  );
  const routeGroups = useMemo(() => {
    const groups = new Map<string, GatewayRoute[]>();
    for (const route of data.routes) {
      groups.set(route.gatewayModelId, [
        ...(groups.get(route.gatewayModelId) ?? []),
        route,
      ]);
    }
    for (const routes of groups.values()) {
      routes.sort((left, right) =>
        left.position === right.position
          ? left.id.localeCompare(right.id)
          : left.position - right.position,
      );
    }
    return [...groups.entries()];
  }, [data.routes]);

  const move = (
    gatewayModelId: string,
    routes: GatewayRoute[],
    index: number,
    offset: -1 | 1,
  ) => {
    const target = index + offset;
    if (target < 0 || target >= routes.length) return;
    const orderedIds = routes.map((route) => route.id);
    [orderedIds[index], orderedIds[target]] = [
      orderedIds[target],
      orderedIds[index],
    ];
    onReorder(gatewayModelId, orderedIds);
  };

  return (
    <div className="space-y-4">
      {routeGroups.length === 0 ? (
        <Card>
          <CardContent>
            <EmptyState text="暂无路由候选" />
          </CardContent>
        </Card>
      ) : (
        routeGroups.map(([gatewayModelId, routes]) => {
          const model = modelById.get(gatewayModelId);
          return (
            <Card key={gatewayModelId}>
              <CardHeader>
                <CardTitle className="flex items-center gap-2 text-base">
                  <ListTree className="h-5 w-5" />
                  {model?.displayName || model?.modelId || gatewayModelId}
                </CardTitle>
              </CardHeader>
              <CardContent className="space-y-2">
                {routes.map((route, index) => {
                  const upstream = upstreamById.get(route.upstreamId);
                  const health = healthByRoute.get(route.id);
                  return (
                    <div
                      key={route.id}
                      className="grid gap-3 rounded-lg border p-3 md:grid-cols-[auto_1fr_auto] md:items-center"
                    >
                      <div className="flex items-center gap-1">
                        <Button
                          variant="ghost"
                          size="icon"
                          disabled={busy || index === 0}
                          aria-label="上移路由候选"
                          onClick={() =>
                            move(gatewayModelId, routes, index, -1)
                          }
                        >
                          <ArrowUp className="h-4 w-4" />
                        </Button>
                        <Button
                          variant="ghost"
                          size="icon"
                          disabled={busy || index === routes.length - 1}
                          aria-label="下移路由候选"
                          onClick={() => move(gatewayModelId, routes, index, 1)}
                        >
                          <ArrowDown className="h-4 w-4" />
                        </Button>
                      </div>
                      <div className="min-w-0">
                        <div className="flex flex-wrap items-center gap-2">
                          <p className="font-medium">
                            {upstream?.name ?? route.upstreamId}
                          </p>
                          <HealthBadge health={health} />
                        </div>
                        <p className="truncate font-mono text-xs text-muted-foreground">
                          {route.targetModel}
                        </p>
                        {health?.lastError && (
                          <p className="mt-1 line-clamp-2 text-xs text-destructive">
                            {health.lastError}
                          </p>
                        )}
                      </div>
                      <div className="flex items-center justify-between gap-2 md:justify-end">
                        <span className="text-xs text-muted-foreground">
                          优先级 {index + 1}
                        </span>
                        <Switch
                          checked={route.enabled}
                          disabled={busy}
                          onCheckedChange={() => onToggle(route)}
                        />
                      </div>
                    </div>
                  );
                })}
              </CardContent>
            </Card>
          );
        })
      )}
    </div>
  );
}

function ObservabilityPage({
  data,
  status,
}: {
  data: GatewayData;
  status: ProxyStatus;
}) {
  return (
    <div className="space-y-5">
      <section className="grid gap-4 md:grid-cols-2 xl:grid-cols-4">
        <StatusCard
          icon={<Gauge className="h-5 w-5" />}
          title="总请求"
          value={`${status.total_requests}`}
          detail={`活跃连接 ${status.active_connections}`}
        />
        <StatusCard
          icon={<Activity className="h-5 w-5" />}
          title="成功率"
          value={`${status.success_rate.toFixed(1)}%`}
          detail={`成功 ${status.success_requests}`}
        />
        <StatusCard
          icon={<CircleAlert className="h-5 w-5" />}
          title="失败请求"
          value={`${status.failed_requests}`}
          detail={`故障转移 ${status.failover_count}`}
        />
        <StatusCard
          icon={<Waypoints className="h-5 w-5" />}
          title="健康记录"
          value={`${data.routeHealth.length}`}
          detail={`迁移问题 ${data.migrationIssues.length}`}
        />
      </section>

      <section className="grid gap-5 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>路由健康</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            {data.routeHealth.length === 0 ? (
              <EmptyState text="尚无路由健康记录" />
            ) : (
              data.routeHealth.map((item) => (
                <div
                  key={item.routeTargetId}
                  className="flex items-start justify-between gap-3 rounded-lg border p-3"
                >
                  <div className="min-w-0">
                    <p className="truncate font-mono text-xs">
                      {item.routeTargetId}
                    </p>
                    <p className="mt-1 text-xs text-muted-foreground">
                      连续失败 {item.consecutiveFailures} · 连续成功{" "}
                      {item.consecutiveSuccesses}
                    </p>
                    {item.lastError && (
                      <p className="mt-1 line-clamp-2 text-xs text-destructive">
                        {item.lastError}
                      </p>
                    )}
                  </div>
                  <HealthBadge health={item} />
                </div>
              ))
            )}
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>迁移报告</CardTitle>
          </CardHeader>
          <CardContent className="space-y-2">
            {data.migrationIssues.length === 0 ? (
              <EmptyState text="没有待处理的迁移问题" />
            ) : (
              data.migrationIssues.map((issue) => (
                <MigrationIssueRow key={issue.migrationKey} issue={issue} />
              ))
            )}
          </CardContent>
        </Card>
      </section>
    </div>
  );
}

function SettingsPage({
  config,
  status,
  auth,
  busy,
  keyName,
  createdSecret,
  onConfigChange,
  onSave,
  onKeyNameChange,
  onCreateKey,
  onRevokeKey,
  onCopy,
}: {
  config: GatewayDomainConfig | null;
  status: ProxyStatus;
  auth: GatewayAuthStatus;
  busy: boolean;
  keyName: string;
  createdSecret: string | null;
  onConfigChange: (config: GatewayDomainConfig) => void;
  onSave: (config: GatewayDomainConfig) => void;
  onKeyNameChange: (value: string) => void;
  onCreateKey: () => void;
  onRevokeKey: (keyId: string) => void;
  onCopy: (value: string, message?: string) => Promise<void>;
}) {
  if (!config) {
    return (
      <Card>
        <CardContent>
          <EmptyState text="正在加载网关设置" />
        </CardContent>
      </Card>
    );
  }

  const setNumber = (key: keyof GatewayDomainConfig, value: string) => {
    onConfigChange({ ...config, [key]: Number(value) });
  };

  return (
    <div className="grid gap-5 xl:grid-cols-2">
      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <Settings2 className="h-5 w-5" />
            网关配置
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          <div className="grid gap-4 sm:grid-cols-2">
            <Field label="监听地址">
              <Input value="127.0.0.1" readOnly disabled />
              <p className="text-xs text-muted-foreground">
                固定为回环地址，不提供局域网或公网监听。
              </p>
            </Field>
            <Field label="监听端口">
              <Input
                type="number"
                min={1}
                max={65535}
                value={config.listenPort}
                disabled={status.running}
                onChange={(event) =>
                  setNumber("listenPort", event.target.value)
                }
              />
            </Field>
            <Field label="最大重试次数">
              <Input
                type="number"
                min={0}
                value={config.maxRetries}
                onChange={(event) =>
                  setNumber("maxRetries", event.target.value)
                }
              />
            </Field>
            <Field label="首包超时（秒）">
              <Input
                type="number"
                min={1}
                value={config.streamingFirstByteTimeout}
                onChange={(event) =>
                  setNumber("streamingFirstByteTimeout", event.target.value)
                }
              />
            </Field>
            <Field label="流式空闲超时（秒）">
              <Input
                type="number"
                min={1}
                value={config.streamingIdleTimeout}
                onChange={(event) =>
                  setNumber("streamingIdleTimeout", event.target.value)
                }
              />
            </Field>
            <Field label="非流式超时（秒）">
              <Input
                type="number"
                min={1}
                value={config.nonStreamingTimeout}
                onChange={(event) =>
                  setNumber("nonStreamingTimeout", event.target.value)
                }
              />
            </Field>
            <Field label="熔断失败阈值">
              <Input
                type="number"
                min={1}
                value={config.circuitFailureThreshold}
                onChange={(event) =>
                  setNumber("circuitFailureThreshold", event.target.value)
                }
              />
            </Field>
            <Field label="熔断恢复成功阈值">
              <Input
                type="number"
                min={1}
                value={config.circuitSuccessThreshold}
                onChange={(event) =>
                  setNumber("circuitSuccessThreshold", event.target.value)
                }
              />
            </Field>
            <Field label="熔断恢复等待（秒）">
              <Input
                type="number"
                min={1}
                value={config.circuitTimeoutSeconds}
                onChange={(event) =>
                  setNumber("circuitTimeoutSeconds", event.target.value)
                }
              />
            </Field>
            <Field label="熔断最少请求数">
              <Input
                type="number"
                min={1}
                value={config.circuitMinRequests}
                onChange={(event) =>
                  setNumber("circuitMinRequests", event.target.value)
                }
              />
            </Field>
            <Field label="熔断错误率阈值">
              <Input
                type="number"
                min={0}
                max={1}
                step="0.01"
                value={config.circuitErrorRateThreshold}
                onChange={(event) =>
                  setNumber("circuitErrorRateThreshold", event.target.value)
                }
              />
            </Field>
          </div>

          <ToggleSetting
            title="要求 Bearer 鉴权"
            description="所有模型协议入口始终要求本地 API Key，不能关闭。"
            checked
            disabled
            onCheckedChange={() => undefined}
          />
          <ToggleSetting
            title="记录网关请求"
            description="只记录经过本地网关的请求，不读取客户端日志或会话数据库。"
            checked={config.enableLogging}
            onCheckedChange={(enableLogging) =>
              onConfigChange({ ...config, enableLogging })
            }
          />

          {status.running && (
            <p className="text-xs text-amber-600 dark:text-amber-400">
              网关运行期间不能修改监听端口；其他配置保存后由后端按运行时契约生效。
            </p>
          )}
          <Button disabled={busy} onClick={() => onSave(config)}>
            <Save className="mr-2 h-4 w-4" />
            保存网关设置
          </Button>
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle className="flex items-center gap-2">
            <KeyRound className="h-5 w-5" />
            API Key
          </CardTitle>
        </CardHeader>
        <CardContent className="space-y-4">
          {!auth.authRequired && (
            <Alert variant="destructive">
              <CircleAlert className="h-4 w-4" />
              <AlertTitle>鉴权当前已关闭</AlertTitle>
              <AlertDescription>
                本机任意进程都可调用网关。建议启用 Bearer 鉴权。
              </AlertDescription>
            </Alert>
          )}
          <div className="flex gap-2">
            <Input
              value={keyName}
              placeholder="API Key 名称"
              onChange={(event) => onKeyNameChange(event.target.value)}
            />
            <Button disabled={busy} onClick={onCreateKey}>
              创建
            </Button>
          </div>

          {createdSecret && (
            <div className="rounded-lg border border-amber-300 bg-amber-50 p-3 dark:border-amber-800 dark:bg-amber-950/30">
              <p className="text-sm font-medium">密钥仅显示一次</p>
              <div className="mt-2 flex gap-2">
                <Input
                  readOnly
                  value={createdSecret}
                  className="font-mono text-xs"
                />
                <Button
                  variant="outline"
                  size="icon"
                  onClick={() => onCopy(createdSecret, "API Key 已复制")}
                >
                  <Copy className="h-4 w-4" />
                </Button>
              </div>
            </div>
          )}

          <div className="space-y-2">
            {auth.keys.length === 0 ? (
              <EmptyState text="尚未创建 API Key" />
            ) : (
              auth.keys.map((key) => (
                <div
                  key={key.id}
                  className="flex items-center justify-between gap-3 rounded-lg border p-3"
                >
                  <div>
                    <p className="text-sm font-medium">{key.name}</p>
                    <p className="font-mono text-xs text-muted-foreground">
                      {key.keyPrefix}…
                    </p>
                  </div>
                  <Button
                    variant="destructive"
                    size="sm"
                    disabled={busy || Boolean(key.revokedAt)}
                    onClick={() => onRevokeKey(key.id)}
                  >
                    {key.revokedAt ? "已撤销" : "撤销"}
                  </Button>
                </div>
              ))
            )}
          </div>
        </CardContent>
      </Card>

      <Alert className="xl:col-span-2">
        <CircleAlert className="h-4 w-4" />
        <AlertTitle>外部历史备份需要手动处理</AlertTitle>
        <AlertDescription>
          Agent Switch 会清理本机数据库、WAL/SHM/freelist
          以及应用自有的旧数据库备份；但无法证明你此前上传到 WebDAV、S3
          或其他外部存储的旧 v2 同步包已经删除。旧包可能包含客户端域、Skills
          或本机凭据材料，请登录对应存储服务自行删除。当前 portable gateway v3
          同步只包含上游、模型、别名与路由图，默认不上传凭据。
        </AlertDescription>
      </Alert>
    </div>
  );
}

function AccessDocsPage({
  gatewayUrl,
  onCopy,
}: {
  gatewayUrl: string;
  onCopy: (value: string, message?: string) => Promise<void>;
}) {
  const endpoints = [
    ["网关地址", gatewayUrl],
    ["Anthropic Messages", `${gatewayUrl}/v1/messages`],
    ["OpenAI Chat Completions", `${gatewayUrl}/v1/chat/completions`],
    ["OpenAI Responses", `${gatewayUrl}/v1/responses`],
    ["Gemini", `${gatewayUrl}/v1beta`],
  ] as const;

  return (
    <div className="space-y-5">
      <Alert>
        <BookOpen className="h-4 w-4" />
        <AlertTitle>只读接入说明</AlertTitle>
        <AlertDescription>
          以下内容只用于复制。Agent Switch
          不检测客户端、不写配置文件，也不会自动应用这些设置。
        </AlertDescription>
      </Alert>

      <Card>
        <CardHeader>
          <CardTitle>协议入口</CardTitle>
        </CardHeader>
        <CardContent className="space-y-3">
          {endpoints.map(([label, value]) => (
            <CopyRow key={label} label={label} value={value} onCopy={onCopy} />
          ))}
        </CardContent>
      </Card>

      <Card>
        <CardHeader>
          <CardTitle>鉴权请求头</CardTitle>
        </CardHeader>
        <CardContent>
          <CopyRow
            label="Authorization"
            value="Authorization: Bearer <你的 Agent Switch API Key>"
            onCopy={onCopy}
          />
          <p className="mt-3 text-xs text-muted-foreground">
            请在客户端侧自行设置。不要把 API Key 写入截图、日志或公开脚本。
          </p>
        </CardContent>
      </Card>
    </div>
  );
}

function StatusCard({
  icon,
  title,
  value,
  detail,
}: {
  icon: React.ReactNode;
  title: string;
  value: string;
  detail: string;
}) {
  return (
    <Card>
      <CardContent className="flex items-start gap-3 p-5">
        <div className="rounded-lg bg-blue-500/10 p-2 text-blue-500">
          {icon}
        </div>
        <div>
          <p className="text-sm text-muted-foreground">{title}</p>
          <p className="text-lg font-semibold">{value}</p>
          <p className="text-xs text-muted-foreground">{detail}</p>
        </div>
      </CardContent>
    </Card>
  );
}

function Metric({ label, value }: { label: string; value: number }) {
  return (
    <div className="rounded-lg border p-3">
      <p className="text-xs text-muted-foreground">{label}</p>
      <p className="text-xl font-semibold">{value}</p>
    </div>
  );
}

function SummaryLine({ label, value }: { label: string; value: string }) {
  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border p-3">
      <span className="text-muted-foreground">{label}</span>
      <span className="font-medium">{value}</span>
    </div>
  );
}

function Field({
  label,
  children,
}: {
  label: string;
  children: React.ReactNode;
}) {
  return (
    <div className="space-y-2">
      <Label>{label}</Label>
      {children}
    </div>
  );
}

function ToggleSetting({
  title,
  description,
  checked,
  disabled = false,
  onCheckedChange,
}: {
  title: string;
  description: string;
  checked: boolean;
  disabled?: boolean;
  onCheckedChange: (checked: boolean) => void;
}) {
  return (
    <div className="flex items-center justify-between gap-4 rounded-lg border p-3">
      <div>
        <p className="font-medium">{title}</p>
        <p className="text-xs text-muted-foreground">{description}</p>
      </div>
      <Switch
        checked={checked}
        disabled={disabled}
        onCheckedChange={onCheckedChange}
      />
    </div>
  );
}

function HealthBadge({ health }: { health: GatewayRouteHealth | undefined }) {
  const state = health?.state ?? "unknown";
  const label =
    state === "closed"
      ? "健康"
      : state === "open"
        ? "熔断"
        : state === "half_open"
          ? "探测中"
          : "未知";
  const variant = state === "open" ? "destructive" : "outline";
  return <Badge variant={variant}>{label}</Badge>;
}

function MigrationIssueRow({ issue }: { issue: GatewayMigrationIssue }) {
  return (
    <div className="rounded-lg border p-3">
      <div className="flex flex-wrap items-center gap-2">
        <Badge variant={issue.severity === "error" ? "destructive" : "outline"}>
          {issue.severity}
        </Badge>
        <p className="font-mono text-xs">{issue.code}</p>
      </div>
      <p className="mt-1 text-xs text-muted-foreground">
        {issue.entityType}
        {issue.legacyAppType ? ` · ${issue.legacyAppType}` : ""}
        {issue.legacyEntityId ? ` · ${issue.legacyEntityId}` : ""}
      </p>
    </div>
  );
}

function EmptyState({ text }: { text: string }) {
  return (
    <p className="py-8 text-center text-sm text-muted-foreground">{text}</p>
  );
}

function CopyRow({
  label,
  value,
  onCopy,
}: {
  label: string;
  value: string;
  onCopy: (value: string, message?: string) => Promise<void>;
}) {
  return (
    <div className="grid gap-2 md:grid-cols-[190px_1fr_auto] md:items-center">
      <Label>{label}</Label>
      <Input readOnly value={value} className="font-mono text-xs" />
      <Button
        variant="outline"
        size="icon"
        onClick={() => onCopy(value, `${label}已复制`)}
      >
        <Copy className="h-4 w-4" />
      </Button>
    </div>
  );
}

function protocolLabel(protocol: string): string {
  const labels: Record<string, string> = {
    anthropic: "Anthropic Messages",
    anthropic_messages: "Anthropic Messages",
    openai_chat: "OpenAI Chat",
    openai_chat_completions: "OpenAI Chat",
    openai_responses: "OpenAI Responses",
    gemini: "Gemini",
    gemini_generate_content: "Gemini",
  };
  return labels[protocol] ?? protocol;
}
