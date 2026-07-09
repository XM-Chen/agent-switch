import { useEffect, useState } from 'react';
import { useMutation, useQueryClient } from '@tanstack/react-query';
import {
  deepLinkApi,
  type DeepLinkImportResult,
  type DeepLinkPreview,
  type DeepLinkResource,
} from '../../lib/api';

interface DeepLinkImportDialogProps {
  initialUrl?: string;
  onClose: () => void;
}

const RESOURCE_QUERY_KEYS: Record<DeepLinkResource, string[]> = {
  provider: ['providers', 'endpoints'],
  prompt: ['prompts'],
  mcp: ['mcp'],
  skill: ['skills'],
};

export function DeepLinkImportDialog({ initialUrl = '', onClose }: DeepLinkImportDialogProps) {
  const queryClient = useQueryClient();
  const [url, setUrl] = useState(initialUrl);
  const [preview, setPreview] = useState<DeepLinkPreview | null>(null);
  const [result, setResult] = useState<DeepLinkImportResult | null>(null);

  const previewMutation = useMutation({
    mutationFn: (value: string) => deepLinkApi.preview(value),
    onSuccess: (data) => {
      setPreview(data);
      setResult(null);
    },
  });

  const importMutation = useMutation({
    mutationFn: (value: string) => deepLinkApi.import(value),
    onSuccess: (data) => {
      setResult(data);
      for (const key of RESOURCE_QUERY_KEYS[data.resource] ?? []) {
        void queryClient.invalidateQueries({ queryKey: [key] });
      }
    },
  });

  useEffect(() => {
    if (initialUrl.trim()) previewMutation.mutate(initialUrl.trim());
  }, [initialUrl]);

  function handlePreview() {
    const value = url.trim();
    if (!value) return;
    previewMutation.mutate(value);
  }

  function handleImport() {
    const value = url.trim();
    if (!value || preview?.blocked) return;
    importMutation.mutate(value);
  }

  const busy = previewMutation.isPending || importMutation.isPending;
  const error = previewMutation.error ?? importMutation.error;

  return (
    <div
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4"
      onClick={onClose}
    >
      <div
        className="w-full max-w-3xl max-h-[88vh] overflow-y-auto rounded-lg bg-white dark:bg-gray-900 border border-gray-200 dark:border-gray-800 p-5 space-y-4 shadow-lg"
        onClick={(e) => e.stopPropagation()}
      >
        <div className="flex items-center justify-between gap-3">
          <div>
            <h2 className="font-semibold text-lg">Deep Link 导入确认</h2>
            <p className="text-xs text-gray-500 mt-1">
              仅支持 ccswitch://v1/import。取消或只预览不会写入数据库，也不会联网。
            </p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="text-xs opacity-70 hover:opacity-100"
            aria-label="关闭"
          >
            ✕
          </button>
        </div>

        <div className="space-y-2">
          <label className="text-sm font-medium" htmlFor="deeplink-url">
            Deep Link URL
          </label>
          <textarea
            id="deeplink-url"
            value={url}
            onChange={(event) => {
              setUrl(event.target.value);
              setPreview(null);
              setResult(null);
            }}
            rows={4}
            className="w-full rounded-md border border-gray-300 dark:border-gray-700 bg-white dark:bg-gray-950 px-3 py-2 text-sm font-mono"
            placeholder="ccswitch://v1/import?resource=provider&..."
          />
          <button
            type="button"
            onClick={handlePreview}
            disabled={busy || !url.trim()}
            className="px-3 py-1.5 rounded-md text-sm bg-gray-100 dark:bg-gray-800 hover:bg-gray-200 dark:hover:bg-gray-700 disabled:opacity-50"
          >
            {previewMutation.isPending ? '解析中...' : '解析预览'}
          </button>
        </div>

        {error && <p className="text-sm text-red-500">操作失败: {error.message}</p>}

        {preview && (
          <div className="space-y-3 rounded-md border border-gray-200 dark:border-gray-800 p-4">
            <div className="flex items-center justify-between gap-3 flex-wrap">
              <div>
                <p className="text-sm font-semibold">资源类型：{preview.resource_label}</p>
                <p className="text-xs text-gray-500 break-all mt-1">{preview.redacted_url}</p>
              </div>
              {preview.blocked && (
                <span className="text-xs px-2 py-1 rounded-full bg-red-100 dark:bg-red-900/30 text-red-700 dark:text-red-300">
                  暂不可导入
                </span>
              )}
            </div>

            {preview.fields.length > 0 && (
              <dl className="grid grid-cols-1 sm:grid-cols-2 gap-2 text-sm">
                {preview.fields.map((field) => (
                  <div key={field.label} className="rounded bg-gray-50 dark:bg-gray-800/50 p-2">
                    <dt className="text-xs text-gray-500">{field.label}</dt>
                    <dd className="break-all">
                      {field.sensitive && field.label === 'API Key' ? '已提供（已遮蔽）' : field.value}
                    </dd>
                  </div>
                ))}
              </dl>
            )}

            {preview.actions.length > 0 && (
              <div className="text-sm">
                <p className="font-medium mb-1">确认后将执行</p>
                <ul className="list-disc pl-5 text-gray-600 dark:text-gray-300 space-y-1">
                  {preview.actions.map((action) => (
                    <li key={action}>{action}</li>
                  ))}
                </ul>
              </div>
            )}

            {preview.warnings.length > 0 && (
              <div className="text-sm rounded-md bg-yellow-50 dark:bg-yellow-900/20 text-yellow-800 dark:text-yellow-200 p-3 space-y-1">
                {preview.warnings.map((warning) => (
                  <p key={warning}>{warning}</p>
                ))}
              </div>
            )}
          </div>
        )}

        {result && (
          <div className="rounded-md bg-gray-50 dark:bg-gray-800/50 p-3 text-sm space-y-2">
            <p>
              新建/更新: {result.created.length} 项；跳过: {result.skipped.length} 项；错误: {result.errors.length} 项
            </p>
            {result.created.length > 0 && <ResultList items={result.created} className="text-green-700 dark:text-green-400" />}
            {result.skipped.length > 0 && <ResultList items={result.skipped} className="text-gray-600 dark:text-gray-300" />}
            {result.warnings.map((warning) => (
              <p key={warning} className="text-yellow-700 dark:text-yellow-300">{warning}</p>
            ))}
            {result.errors.map((err) => (
              <p key={err} className="text-red-600 dark:text-red-400">{err}</p>
            ))}
          </div>
        )}

        <div className="flex justify-end gap-2 pt-2">
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 rounded-md text-sm border border-gray-300 dark:border-gray-700"
          >
            取消
          </button>
          <button
            type="button"
            onClick={handleImport}
            disabled={!preview || preview.blocked || busy}
            className="px-4 py-2 rounded-md text-sm bg-blue-600 text-white hover:bg-blue-700 disabled:opacity-50"
          >
            {importMutation.isPending ? '导入中...' : '确认导入'}
          </button>
        </div>
      </div>
    </div>
  );
}

function ResultList({ items, className }: { items: DeepLinkImportResult['created']; className: string }) {
  return (
    <ul className={`list-disc pl-5 ${className}`}>
      {items.map((item, index) => (
        <li key={`${item.kind}-${item.id ?? index}`}>
          {item.kind}: {item.name}{item.message ? `（${item.message}）` : ''}
        </li>
      ))}
    </ul>
  );
}
