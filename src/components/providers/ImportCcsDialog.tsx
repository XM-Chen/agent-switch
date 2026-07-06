import { useEffect, useMemo, useState } from 'react';
import { useMutation } from '@tanstack/react-query';
import {
  ccsImportApi,
  type CcsDetectItem,
  type CcsImportItem,
  type CcsImportResponse,
} from '../../lib/api';

interface ImportCcsDialogProps {
  onClose: () => void;
  /** 导入成功后由父层 invalidate provider 列表。 */
  onImported: () => void;
}

type BannerKind = 'success' | 'warning' | 'error';
interface Banner {
  kind: BannerKind;
  text: string;
}

/**
 * 从本地 ccs 一键导入 Claude 渠道对话框。
 *
 * 打开时调 detect 拿预览列表 → 用户勾选 → 调 import 批量建 endpoint+provider。
 * 空 base_url（官方登录）与已导入项默认不勾选；冲突项展示导入后名称。
 */
export function ImportCcsDialog({ onClose, onImported }: ImportCcsDialogProps) {
  const [detecting, setDetecting] = useState(true);
  const [detectError, setDetectError] = useState<string | null>(null);
  const [items, setItems] = useState<CcsDetectItem[]>([]);
  const [found, setFound] = useState(false);
  const [configPath, setConfigPath] = useState('');
  const [source, setSource] = useState('');

  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [result, setResult] = useState<CcsImportResponse | null>(null);
  const [banner, setBanner] = useState<Banner | null>(null);

  // 打开即探测。
  useEffect(() => {
    let cancelled = false;
    setDetecting(true);
    setDetectError(null);
    ccsImportApi
      .detect()
      .then((resp) => {
        if (cancelled) return;
        setFound(resp.found);
        setConfigPath(resp.config_path);
        setSource(resp.source);
        setItems(resp.providers);
        // 默认勾选：importable 且未 already_imported。冲突项仍默认勾选（用户可取消）。
        const defaultSel = new Set<string>(
          resp.providers
            .filter((p) => p.importable && !p.already_imported)
            .map((p) => p.original_id),
        );
        setSelected(defaultSel);
      })
      .catch((e: Error) => {
        if (cancelled) return;
        setDetectError(e.message);
      })
      .finally(() => {
        if (!cancelled) setDetecting(false);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const importMutation = useMutation({
    mutationFn: (reqItems: CcsImportItem[]) => ccsImportApi.import(reqItems),
    onSuccess: (data) => {
      setResult(data);
      const errCount = data.errors.length;
      const createdCount = data.created_providers.length;
      if (errCount > 0 && createdCount > 0) {
        setBanner({ kind: 'warning', text: `导入 ${createdCount} 项成功，${errCount} 项失败` });
      } else if (errCount > 0) {
        setBanner({ kind: 'error', text: `导入失败：${errCount} 项` });
      } else if (createdCount === 0) {
        setBanner({ kind: 'warning', text: '未导入任何项（已全部跳过）' });
      } else {
        setBanner({ kind: 'success', text: `导入成功：${createdCount} 项` });
      }
      if (createdCount > 0) onImported();
    },
    onError: (e: Error) => {
      setBanner({ kind: 'error', text: `导入失败: ${e.message}` });
    },
  });

  function toggle(id: string) {
    setSelected((prev) => {
      const next = new Set(prev);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
  }

  const selectedItems = useMemo(
    () => items.filter((p) => selected.has(p.original_id)),
    [items, selected],
  );

  function handleConfirm() {
    const payload: CcsImportItem[] = selectedItems.map((p) => ({
      original_id: p.original_id,
      imported_name: p.imported_name,
    }));
    importMutation.mutate(payload);
  }

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onClick={onClose}
    >
      <div
        className="w-full max-w-2xl max-h-[85vh] overflow-y-auto rounded-lg bg-white dark:bg-gray-900 border border-gray-200 dark:border-gray-800 p-5 space-y-3 shadow-lg"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between">
          <h2 className="font-semibold text-lg">从 ccs 导入</h2>
          <button
            type="button"
            onClick={onClose}
            className="text-xs opacity-70 hover:opacity-100"
            aria-label="关闭"
          >
            ✕
          </button>
        </div>

        {detecting && <p className="text-sm text-gray-500">正在探测本地 ccs 安装...</p>}

        {detectError && (
          <p className="text-sm text-red-500">探测失败: {detectError}</p>
        )}

        {!detecting && !detectError && found && (
          <p className="text-xs text-gray-500">
            数据源: <code className="text-xs">{source || '未知'}</code>
            {' · '}
            <code className="text-xs">{configPath}</code>
          </p>
        )}

        {!detecting && !detectError && !found && (
          <p className="text-sm text-gray-500">
            未检测到 ccs 安装（<code className="text-xs">{configPath}</code> 不存在）。
            请先安装 cc-switch 并添加至少一个 Claude 渠道。
          </p>
        )}

        {!detecting && !detectError && found && items.length === 0 && (
          <p className="text-sm text-gray-500">ccs 中没有可导入的 provider。</p>
        )}

        {banner && <BannerView banner={banner} />}

        {!detecting && !detectError && found && items.length > 0 && (
          <>
            <p className="text-xs text-gray-500">
              勾选要导入的 ccs 渠道，导入后即可在切换器页切换生效。
              空 base_url（官方登录）与已导入项默认不勾选。
            </p>
            <ul className="space-y-2">
              {items.map((p) => (
                <li
                  key={p.original_id}
                  className="flex items-start gap-3 p-3 rounded-md border border-gray-200 dark:border-gray-800"
                >
                  <input
                    type="checkbox"
                    checked={selected.has(p.original_id)}
                    onChange={() => toggle(p.original_id)}
                    disabled={!p.importable}
                    className="mt-1"
                  />
                  <div className="flex-1 min-w-0 space-y-1">
                    <div className="flex items-center gap-2 flex-wrap">
                      <span className="font-medium text-sm">{p.name}</span>
                      {p.already_imported && (
                        <Tag className="bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-300">
                          已导入
                        </Tag>
                      )}
                      {p.conflict && !p.already_imported && (
                        <Tag className="bg-yellow-100 dark:bg-yellow-900/30 text-yellow-700 dark:text-yellow-300">
                          冲突 → 导入后名: {p.imported_name}
                        </Tag>
                      )}
                      {!p.importable && (
                        <Tag className="bg-red-100 dark:bg-red-900/30 text-red-700 dark:text-red-300">
                          不可导入
                        </Tag>
                      )}
                      {p.has_api_key ? (
                        <Tag className="bg-green-100 dark:bg-green-900/30 text-green-700 dark:text-green-300">
                          含 API Key
                        </Tag>
                      ) : (
                        <Tag className="bg-gray-100 dark:bg-gray-800 text-gray-500">
                          无 API Key
                        </Tag>
                      )}
                    </div>
                    <p className="text-xs text-gray-500 break-all">
                      {p.base_url ?? '（无 base_url）'}
                    </p>
                    {p.model && (
                      <p className="text-xs text-gray-400">model: {p.model}</p>
                    )}
                    {p.warning && (
                      <p className="text-xs text-yellow-600 dark:text-yellow-400">
                        {p.warning}
                      </p>
                    )}
                  </div>
                </li>
              ))}
            </ul>

            {result && (
              <div className="text-xs space-y-1 p-3 rounded-md bg-gray-50 dark:bg-gray-800/50">
                <p>
                  新建: {result.created_providers.length} 项；跳过: {result.skipped.length} 项；失败: {result.errors.length} 项
                </p>
                {result.created_providers.length > 0 && (
                  <ul className="list-disc pl-4 text-green-700 dark:text-green-400">
                    {result.created_providers.map((c) => (
                      <li key={c.provider_id}>
                        {c.name}（provider: {c.provider_id.slice(0, 8)}…）
                      </li>
                    ))}
                  </ul>
                )}
                {result.skipped.length > 0 && (
                  <ul className="list-disc pl-4 text-gray-500">
                    {result.skipped.map((s, i) => (
                      <li key={i}>
                        {s.original_id.slice(0, 8)}…：{s.reason}
                      </li>
                    ))}
                  </ul>
                )}
                {result.errors.length > 0 && (
                  <ul className="list-disc pl-4 text-red-600 dark:text-red-400">
                    {result.errors.map((e, i) => (
                      <li key={i}>
                        {e.original_id.slice(0, 8)}…：{e.message}
                      </li>
                    ))}
                  </ul>
                )}
              </div>
            )}

            <div className="flex gap-2 justify-end pt-2">
              <button
                type="button"
                onClick={onClose}
                className="px-4 py-2 bg-gray-100 dark:bg-gray-800 rounded-md text-sm hover:bg-gray-200 dark:hover:bg-gray-700"
              >
                关闭
              </button>
              <button
                type="button"
                onClick={handleConfirm}
                disabled={selectedItems.length === 0 || importMutation.isPending}
                className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
              >
                {importMutation.isPending
                  ? '导入中...'
                  : `导入 ${selectedItems.length} 项`}
              </button>
            </div>
          </>
        )}
      </div>
    </div>
  );
}

function Tag({ children, className }: { children: React.ReactNode; className: string }) {
  return (
    <span className={`inline-block px-1.5 py-0.5 rounded text-xs ${className}`}>
      {children}
    </span>
  );
}

function BannerView({ banner }: { banner: Banner }) {
  const styles: Record<BannerKind, string> = {
    success: 'bg-green-50 dark:bg-green-900/20 border-green-300 dark:border-green-700 text-green-700 dark:text-green-300',
    warning: 'bg-yellow-50 dark:bg-yellow-900/20 border-yellow-300 dark:border-yellow-700 text-yellow-700 dark:text-yellow-300',
    error: 'bg-red-50 dark:bg-red-900/20 border-red-300 dark:border-red-700 text-red-700 dark:text-red-300',
  };
  return (
    <div className={`px-3 py-2 rounded-md border text-sm ${styles[banner.kind]}`}>
      {banner.text}
    </div>
  );
}
