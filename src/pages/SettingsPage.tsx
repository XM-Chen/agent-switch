import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { useEffect, useRef, useState } from 'react';
import { settingsApi, portabilityApi } from '../lib/api';
import type { ImportResult } from '../lib/api';
import {
  checkForUpdate,
  getCurrentVersion,
  type DownloadProgress,
  type UpdateInfo,
} from '../lib/updater';

export function SettingsPage() {
  const queryClient = useQueryClient();
  const { data, isLoading, error } = useQuery({
    queryKey: ['auto-refresh'],
    queryFn: settingsApi.getAutoRefresh,
  });

  const toggle = useMutation({
    mutationFn: (enabled: boolean) => settingsApi.setAutoRefresh(enabled),
    onSuccess: () => queryClient.invalidateQueries({ queryKey: ['auto-refresh'] }),
    onError: (e: Error) => alert(`切换失败: ${e.message}`),
  });

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">设置</h1>
        <p className="text-sm text-gray-500 mt-1">应用配置与模型刷新策略</p>
      </div>

      {isLoading && <p className="text-gray-500">加载中...</p>}
      {error && <p className="text-red-500">加载失败: {error.message}</p>}

      {data && (
        <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-5 space-y-4">
          <div>
            <h2 className="font-semibold">模型自动刷新</h2>
            <p className="text-xs text-gray-500 mt-0.5">
              开启后：应用启动时刷新一次上游模型，之后每 6 小时（±随机 30 分钟）自动刷新。关闭时仅手动刷新。
            </p>
          </div>

          <div className="flex items-center justify-between">
            <div>
              <p className="text-sm font-medium">自动刷新</p>
              <p className="text-xs text-gray-500">
                {data.enabled ? '已开启' : '已关闭（默认）'}
              </p>
            </div>
            <button
              onClick={() => toggle.mutate(!data.enabled)}
              disabled={toggle.isPending}
              className={`relative inline-flex h-6 w-11 items-center rounded-full transition-colors ${
                data.enabled ? 'bg-green-600' : 'bg-gray-300 dark:bg-gray-700'
              }`}
            >
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-white transition-transform ${
                  data.enabled ? 'translate-x-6' : 'translate-x-1'
                }`}
              />
            </button>
          </div>

          <div className="border-t border-gray-200 dark:border-gray-800 pt-4 space-y-2 text-sm">
            <div className="flex justify-between">
              <span className="text-gray-500">最近同步时间</span>
              <span className="font-mono text-xs">{data.last_sync_at ?? '从未同步'}</span>
            </div>
            <div className="flex justify-between">
              <span className="text-gray-500">最近同步错误</span>
              <span
                className={`text-xs max-w-md text-right ${
                  data.last_sync_error ? 'text-red-500' : 'text-gray-400'
                }`}
              >
                {data.last_sync_error ?? '无'}
              </span>
            </div>
          </div>
        </div>
      )}

      <PortabilityCard />

      <UpdaterCard />
    </div>
  );
}

// ── 关于与更新卡片 ─────────────────────────────────────

function UpdaterCard() {
  const [currentVersion, setCurrentVersion] = useState<string>('');
  // 检查/下载状态
  const [checking, setChecking] = useState(false);
  const [installing, setInstalling] = useState(false);
  const [update, setUpdate] = useState<UpdateInfo | null>(null);
  const [progress, setProgress] = useState<DownloadProgress | null>(null);
  const [msg, setMsg] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // 组件卸载后不再 setState，避免下载中卸载报警告。
  const mounted = useRef(true);

  useEffect(() => {
    mounted.current = true;
    getCurrentVersion()
      .then((v) => {
        if (mounted.current) setCurrentVersion(v);
      })
      .catch(() => {
        // 取版本失败不阻塞检查更新，留空即可。
      });
    return () => {
      mounted.current = false;
    };
  }, []);

  const handleCheck = async () => {
    setChecking(true);
    setError(null);
    setMsg(null);
    setUpdate(null);
    setProgress(null);
    try {
      const info = await checkForUpdate();
      if (!mounted.current) return;
      setUpdate(info);
      if (!info.available) {
        setMsg('当前已是最新版本。');
      }
    } catch (e) {
      if (mounted.current) setError(e instanceof Error ? e.message : String(e));
    } finally {
      if (mounted.current) setChecking(false);
    }
  };

  const handleInstall = async () => {
    if (!update?.downloadAndInstall) return;
    setInstalling(true);
    setError(null);
    setMsg('正在下载更新...');
    try {
      await update.downloadAndInstall((p) => {
        if (mounted.current) setProgress(p);
      });
      // downloadAndInstall 内部会在安装后重启应用；正常不会执行到这里。
      if (mounted.current) setMsg('更新完成，正在重启...');
    } catch (e) {
      if (mounted.current) {
        setError(e instanceof Error ? e.message : String(e));
        setInstalling(false);
      }
    }
  };

  // 下载进度百分比（内容长度可用时）。
  const percent =
    progress && progress.contentLength
      ? Math.min(100, Math.round((progress.downloaded / progress.contentLength) * 100))
      : null;

  return (
    <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-5 space-y-4">
      <div>
        <h2 className="font-semibold">关于与更新</h2>
        <p className="text-xs text-gray-500 mt-0.5">
          检查是否有新版本，若有可一键下载并安装（下载后自动校验签名，安装完成后重启生效）。
        </p>
      </div>

      <div className="flex items-center justify-between">
        <div>
          <p className="text-sm font-medium">当前版本</p>
          <p className="text-xs text-gray-500 font-mono">
            {currentVersion ? `v${currentVersion}` : '读取中...'}
          </p>
        </div>
        <button
          onClick={handleCheck}
          disabled={checking || installing}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
        >
          {checking ? '检查中...' : '检查更新'}
        </button>
      </div>

      {/* 有新版 */}
      {update?.available && (
        <div className="border-t border-gray-200 dark:border-gray-800 pt-4 space-y-3">
          <div className="bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 rounded-md p-3 space-y-2">
            <p className="text-sm font-medium text-green-700 dark:text-green-300">
              发现新版本 v{update.version}
            </p>
            {update.notes && (
              <div className="text-xs text-green-700 dark:text-green-300 whitespace-pre-wrap">
                {update.notes}
              </div>
            )}
          </div>

          {installing ? (
            <div className="space-y-1">
              <div className="h-2 w-full rounded-full bg-gray-200 dark:bg-gray-700 overflow-hidden">
                <div
                  className="h-full bg-blue-600 transition-all"
                  style={{ width: percent !== null ? `${percent}%` : '40%' }}
                />
              </div>
              <p className="text-xs text-gray-500">
                {percent !== null
                  ? `下载中 ${percent}%`
                  : progress
                    ? `已下载 ${(progress.downloaded / 1024 / 1024).toFixed(1)} MB`
                    : '准备下载...'}
              </p>
            </div>
          ) : (
            <button
              onClick={handleInstall}
              className="px-4 py-2 bg-green-600 text-white rounded-md text-sm hover:bg-green-700 disabled:opacity-50"
            >
              立即更新
            </button>
          )}
        </div>
      )}

      {/* 反馈 */}
      {msg && !error && (
        <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-md p-3 text-sm text-blue-700 dark:text-blue-300">
          {msg}
        </div>
      )}
      {error && (
        <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-md p-3 text-sm text-red-700 dark:text-red-300 whitespace-pre-wrap">
          {error}
        </div>
      )}
    </div>
  );
}

// ── 配置导入导出卡片 ─────────────────────────────────────

/// 生成导出包下载文件名：含 mode + 时间戳，完整备份 .asbak，脱敏 .ascfg。
function buildDownloadName(mode: 'full_backup' | 'portable'): string {
  const ts = new Date()
    .toISOString()
    .replace(/[:T]/g, '-')
    .replace(/\..+/, '');
  const ext = mode === 'full_backup' ? 'asbak' : 'ascfg';
  return `agent-switch-${mode}-${ts}.${ext}`;
}

/// 触发浏览器下载导出包文本。
function downloadPackage(packageText: string, mode: 'full_backup' | 'portable') {
  const blob = new Blob([packageText], { type: 'application/json' });
  const url = URL.createObjectURL(blob);
  const a = document.createElement('a');
  a.href = url;
  a.download = buildDownloadName(mode);
  document.body.appendChild(a);
  a.click();
  document.body.removeChild(a);
  URL.revokeObjectURL(url);
}

function PortabilityCard() {
  const queryClient = useQueryClient();

  // 脱敏导出密码
  const [portablePassword, setPortablePassword] = useState('');
  // 导入：文件名 + 包文本 + 密码
  const [importFileName, setImportFileName] = useState('');
  const [importPackage, setImportPackage] = useState('');
  const [importPassword, setImportPassword] = useState('');
  const fileInputRef = useRef<HTMLInputElement>(null);

  // 导出/导入反馈
  const [exportWarnings, setExportWarnings] = useState<string[] | null>(null);
  const [exportMsg, setExportMsg] = useState<string | null>(null);
  const [importResult, setImportResult] = useState<ImportResult | null>(null);
  const [importError, setImportError] = useState<string | null>(null);

  const exportFull = useMutation({
    mutationFn: () => portabilityApi.exportConfig('full_backup'),
    onSuccess: (result) => {
      downloadPackage(result.package, 'full_backup');
      setExportWarnings(result.warnings.length ? result.warnings : null);
      setExportMsg('完整备份导出成功，已触发下载。');
      setImportResult(null);
      setImportError(null);
    },
    onError: (e: Error) => {
      setExportMsg(`完整备份导出失败：${e.message}`);
      setExportWarnings(null);
    },
  });

  const exportPortable = useMutation({
    mutationFn: () => portabilityApi.exportConfig('portable', portablePassword),
    onSuccess: (result) => {
      downloadPackage(result.package, 'portable');
      setExportWarnings(result.warnings.length ? result.warnings : null);
      setExportMsg('脱敏配置导出成功，已触发下载。');
    },
    onError: (e: Error) => {
      setExportMsg(`脱敏导出失败：${e.message}`);
      setExportWarnings(null);
    },
  });

  const importConfig = useMutation({
    mutationFn: () =>
      portabilityApi.importConfig({
        package: importPackage,
        password: importPassword || undefined,
      }),
    onSuccess: (result) => {
      setImportResult(result);
      setImportError(null);
      // 导入会改变账号/端点/模型等，刷新相关查询缓存。
      queryClient.invalidateQueries();
      // 重置表单，防止用户意外重复导入同一包。
      setImportPackage('');
      setImportFileName('');
      setImportPassword('');
    },
    onError: (e: Error) => {
      setImportError(`导入失败：${e.message}`);
      setImportResult(null);
    },
  });

  // 读取导入文件文本作为 package。
  const handleFileChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const file = e.target.files?.[0];
    if (!file) return;
    setImportFileName(file.name);
    const reader = new FileReader();
    reader.onload = () => {
      setImportPackage(typeof reader.result === 'string' ? reader.result : '');
    };
    reader.onerror = () => {
      setImportError('读取文件失败，请重试。');
    };
    reader.readAsText(file);
  };

  return (
    <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-5 space-y-5">
      <div>
        <h2 className="font-semibold">配置导入导出</h2>
        <p className="text-xs text-gray-500 mt-0.5">
          本机加密完整备份或可迁移脱敏配置，导入后自动接管状态统一关闭、不会写入 Claude Code / Codex 配置。
        </p>
      </div>

      {/* 风险提示 */}
      <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-md p-3 text-xs text-amber-700 dark:text-amber-300 space-y-1">
        <p className="font-medium">风险提示</p>
        <ul className="list-disc list-inside space-y-0.5">
          <li>完整备份包含凭据，绑定本机主密钥，跨机器无法恢复敏感凭据。</li>
          <li>脱敏包跨机器可迁移，但不含凭据，导入后需重新录入 API Key / OAuth 登录。</li>
          <li>完整备份导入会覆盖现有配置，脱敏导入按匹配键合并，请谨慎操作。</li>
          <li>任一导入完成后自动接管状态统一关闭，不会自动写入工具配置。</li>
        </ul>
      </div>

      {/* 完整备份导出 */}
      <div className="space-y-2">
        <p className="text-sm font-medium">完整备份导出（含凭据，绑定本机）</p>
        <p className="text-xs text-gray-500">
          用系统主密钥加密，含账号/端点凭据，适合本机或同凭据环境恢复。主密钥不可用时无法导出。
        </p>
        <button
          onClick={() => exportFull.mutate()}
          disabled={exportFull.isPending}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
        >
          {exportFull.isPending ? '导出中...' : '导出完整备份'}
        </button>
      </div>

      {/* 脱敏导出 */}
      <div className="space-y-2 border-t border-gray-200 dark:border-gray-800 pt-4">
        <p className="text-sm font-medium">脱敏配置导出（跨机器可迁移）</p>
        <p className="text-xs text-gray-500">
          不含 API Key / OAuth token / 日志，设置导出密码（Argon2id 派生密钥）后可跨机器解密。
        </p>
        <div className="flex gap-2 items-center">
          <input
            type="password"
            value={portablePassword}
            onChange={(e) => setPortablePassword(e.target.value)}
            placeholder="设置导出密码"
            className="flex-1 px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-white dark:bg-gray-800"
          />
          <button
            onClick={() => exportPortable.mutate()}
            disabled={exportPortable.isPending || !portablePassword}
            className="px-4 py-2 bg-green-600 text-white rounded-md text-sm hover:bg-green-700 disabled:opacity-50"
          >
            {exportPortable.isPending ? '导出中...' : '导出脱敏配置'}
          </button>
        </div>
      </div>

      {/* 导出反馈 */}
      {exportMsg && (
        <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-md p-3 text-sm text-blue-700 dark:text-blue-300">
          {exportMsg}
        </div>
      )}
      {exportWarnings && exportWarnings.length > 0 && (
        <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-md p-3 text-xs text-amber-700 dark:text-amber-300 space-y-1">
          <p className="font-medium">导出警告</p>
          <ul className="list-disc list-inside">
            {exportWarnings.map((w, i) => (
              <li key={i}>{w}</li>
            ))}
          </ul>
        </div>
      )}

      {/* 导入 */}
      <div className="space-y-2 border-t border-gray-200 dark:border-gray-800 pt-4">
        <p className="text-sm font-medium">导入配置</p>
        <p className="text-xs text-gray-500">
          选择导出包文件（.asbak / .ascfg）。脱敏包需输入导出密码；完整备份可留空（用本机主密钥解密）。
        </p>
        <div className="flex flex-col gap-2">
          <div className="flex gap-2 items-center">
            <input
              ref={fileInputRef}
              type="file"
              accept=".asbak,.ascfg,.json,application/json,text/plain"
              onChange={handleFileChange}
              className="text-sm text-gray-600 dark:text-gray-300 file:mr-2 file:px-3 file:py-1.5 file:rounded-md file:border-0 file:bg-gray-100 dark:file:bg-gray-700 file:text-gray-700 dark:file:text-gray-200"
            />
            {importFileName && (
              <span className="text-xs text-gray-500 truncate">{importFileName}</span>
            )}
          </div>
          <input
            type="password"
            value={importPassword}
            onChange={(e) => setImportPassword(e.target.value)}
            placeholder="导出密码（脱敏包必填，完整备份留空）"
            className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-white dark:bg-gray-800"
          />
          <button
            onClick={() => importConfig.mutate()}
            disabled={importConfig.isPending || !importPackage}
            className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50 self-start"
          >
            {importConfig.isPending ? '导入中...' : '导入配置'}
          </button>
        </div>
      </div>

      {/* 导入反馈 */}
      {importError && (
        <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-md p-3 text-sm text-red-700 dark:text-red-300 whitespace-pre-wrap">
          {importError}
        </div>
      )}
      {importResult && (
        <div className="bg-blue-50 dark:bg-blue-900/20 border border-blue-200 dark:border-blue-800 rounded-md p-3 text-sm text-blue-700 dark:text-blue-300 space-y-2">
          <p className="font-medium">导入完成</p>
          <ul className="text-xs space-y-0.5">
            <li>账号：{importResult.imported.accounts}</li>
            <li>端点：{importResult.imported.endpoints}</li>
            <li>模型：{importResult.imported.endpoint_models}</li>
            <li>别名：{importResult.imported.model_aliases}</li>
            <li>路由设置：{importResult.imported.route_settings}</li>
            <li>自动接管：已全部关闭（{importResult.imported.tool_takeover} 项重置）</li>
          </ul>
          {importResult.pre_import_backup && (
            <p className="text-xs">
              导入前已自动备份当前数据库到：
              <span className="font-mono break-all">
                {importResult.pre_import_backup}
              </span>
            </p>
          )}
          {importResult.warnings.length > 0 && (
            <ul className="text-xs list-disc list-inside">
              {importResult.warnings.map((w, i) => (
                <li key={i}>{w}</li>
              ))}
            </ul>
          )}
        </div>
      )}
    </div>
  );
}
