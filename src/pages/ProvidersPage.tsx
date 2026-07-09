import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useEffect, useMemo, useRef, useState } from 'react';
import {
  commonConfigApi,
  providersApi,
  type CreateProviderBody,
  type Provider,
  type UpdateProviderBody,
} from '../lib/api';
import { AppTypeSection } from '../components/providers/AppTypeSection';
import { ImportCcsDialog } from '../components/providers/ImportCcsDialog';
import { ProviderForm } from '../components/providers/ProviderForm';
import { parseJsonObjectText } from '../components/providers/commonConfigHelpers';
import {
  APP_TYPES,
  groupByAppType,
  moveItem,
  type AppType,
} from './providersUtils';

type BannerKind = 'success' | 'warning' | 'error';
interface Banner {
  kind: BannerKind;
  text: string;
}

/** 自动消失计时器（success/warning 3s 后清，error 常驻）。 */
const BANNER_AUTO_CLEAR_MS = 3000;

export function ProvidersPage() {
  const queryClient = useQueryClient();

  // 两次 list 请求，共享 ['providers'] 前缀便于一并 invalidate。
  const claudeQuery = useQuery({
    queryKey: ['providers', 'claude-code'],
    queryFn: () => providersApi.list('claude-code'),
  });
  const codexQuery = useQuery({
    queryKey: ['providers', 'codex'],
    queryFn: () => providersApi.list('codex'),
  });

  const isLoading = claudeQuery.isLoading || codexQuery.isLoading;
  const error = claudeQuery.error ?? codexQuery.error;

  const grouped = useMemo(
    () => groupByAppType([...(claudeQuery.data ?? []), ...(codexQuery.data ?? [])]),
    [claudeQuery.data, codexQuery.data],
  );

  const [banner, setBanner] = useState<Banner | null>(null);
  const bannerTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  const [formState, setFormState] = useState<{
    open: boolean;
    initial: Provider | null;
  }>({ open: false, initial: null });

  const [formError, setFormError] = useState<string | null>(null);
  const [switchingId, setSwitchingId] = useState<string | null>(null);
  const [importOpen, setImportOpen] = useState(false);

  function clearBannerTimer() {
    if (bannerTimer.current) {
      clearTimeout(bannerTimer.current);
      bannerTimer.current = null;
    }
  }

  function showBanner(kind: BannerKind, text: string) {
    clearBannerTimer();
    setBanner({ kind, text });
    if (kind !== 'error') {
      bannerTimer.current = setTimeout(() => {
        setBanner(null);
        bannerTimer.current = null;
      }, BANNER_AUTO_CLEAR_MS);
    }
  }

  // ── 切换 ──────────────────────────────────────────────
  const switchMutation = useMutation({
    mutationFn: (id: string) => {
      setSwitchingId(id);
      return providersApi.switch(id);
    },
    onSuccess: (data) => {
      if (data.warnings.length > 0) {
        showBanner('warning', `切换成功，但: ${data.warnings.join('；')}`);
      } else {
        showBanner('success', '切换成功');
      }
    },
    onError: (e: Error) => {
      showBanner('error', `切换失败: ${e.message}`);
    },
    onSettled: () => {
      setSwitchingId(null);
      void queryClient.invalidateQueries({ queryKey: ['providers'] });
    },
  });

  // ── 排序 ──────────────────────────────────────────────
  const reorderMutation = useMutation({
    mutationFn: (items: { id: string; sort_index: number }[]) =>
      providersApi.reorder(items),
    onSuccess: () => {
      // 排序成功无需 banner，静默刷新即可。
    },
    onError: (e: Error) => {
      showBanner('error', `排序失败: ${e.message}`);
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ['providers'] });
    },
  });

  function handleMove(appType: AppType, from: number, to: number) {
    const items = grouped[appType];
    if (items.length === 0) return;
    const payload = moveItem(items, from, to);
    reorderMutation.mutate(payload);
  }

  // ── 创建/更新 ─────────────────────────────────────────
  const upsertMutation = useMutation({
    mutationFn: async (args: {
      body: CreateProviderBody | UpdateProviderBody;
      isEdit: boolean;
      id?: string;
    }): Promise<void> => {
      if (args.isEdit && args.id) {
        await providersApi.update(args.id, args.body as UpdateProviderBody);
        return;
      }
      await providersApi.create(args.body as CreateProviderBody);
    },
    onSuccess: () => {
      setFormError(null);
      setFormState({ open: false, initial: null });
      showBanner('success', '保存成功');
    },
    onError: (e: Error) => {
      setFormError(e.message);
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ['providers'] });
    },
  });

  function handleFormSubmit(body: CreateProviderBody | UpdateProviderBody, isEdit: boolean) {
    upsertMutation.mutate({ body, isEdit, id: formState.initial?.id });
  }

  // ── 删除（二次确认）──────────────────────────────────
  const deleteMutation = useMutation({
    mutationFn: (id: string) => providersApi.remove(id),
    onSuccess: () => {
      showBanner('success', '已删除');
    },
    onError: (e: Error) => {
      showBanner('error', `删除失败: ${e.message}`);
    },
    onSettled: () => {
      void queryClient.invalidateQueries({ queryKey: ['providers'] });
    },
  });

  function handleDelete(provider: Provider) {
    if (confirm(`确定删除 provider "${provider.name}"？`)) {
      deleteMutation.mutate(provider.id);
    }
  }

  return (
    <div className="space-y-6">
      <div className="flex items-center justify-between">
        <div>
          <h1 className="text-2xl font-bold">切换器</h1>
          <p className="text-sm text-gray-500 mt-1">
            按 app_type 分组管理 provider，点一下即切。
          </p>
        </div>
        <div className="flex items-center gap-2">
          <button
            type="button"
            onClick={() => setImportOpen(true)}
            className="px-4 py-2 bg-gray-100 dark:bg-gray-800 text-gray-700 dark:text-gray-200 rounded-md text-sm hover:bg-gray-200 dark:hover:bg-gray-700"
          >
            从 ccs 导入
          </button>
          <button
            type="button"
            onClick={() => {
              setFormError(null);
              setFormState({ open: true, initial: null });
            }}
            className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700"
          >
            添加 provider
          </button>
        </div>
      </div>

      {banner && <BannerView banner={banner} onDismiss={() => setBanner(null)} />}

      <CommonConfigCard onSaved={(text) => showBanner('success', text)} />

      {isLoading && <p className="text-gray-500">加载中...</p>}
      {error && <p className="text-red-500">加载失败: {error.message}</p>}

      {!isLoading && !error && (
        <div className="space-y-8">
          {APP_TYPES.map((appType) => (
            <AppTypeSection
              key={appType}
              appType={appType}
              providers={grouped[appType]}
              onSwitch={(id) => switchMutation.mutate(id)}
              onEdit={(p) => {
                setFormError(null);
                setFormState({ open: true, initial: p });
              }}
              onDelete={handleDelete}
              onMove={handleMove}
              switchingId={switchingId}
              movePending={reorderMutation.isPending}
            />
          ))}
        </div>
      )}

      {formState.open && (
        <ProviderForm
          initial={formState.initial}
          onSubmit={handleFormSubmit}
          onCancel={() => {
            setFormError(null);
            setFormState({ open: false, initial: null });
          }}
          pending={upsertMutation.isPending}
          error={formError}
          onApplyLive={() => {
            if (formState.initial) switchMutation.mutate(formState.initial.id);
          }}
          applyLivePending={switchMutation.isPending}
        />
      )}

      {importOpen && (
        <ImportCcsDialog
          onClose={() => setImportOpen(false)}
          onImported={() => {
            void queryClient.invalidateQueries({ queryKey: ['providers'] });
            showBanner('success', '从 ccs 导入完成，列表已刷新');
          }}
        />
      )}
    </div>
  );
}

function CommonConfigCard({ onSaved }: { onSaved: (text: string) => void }) {
  const queryClient = useQueryClient();
  const query = useQuery({
    queryKey: ['common-config', 'claude-code'],
    queryFn: () => commonConfigApi.get('claude-code'),
  });
  const [text, setText] = useState('');
  const [jsonError, setJsonError] = useState<string | null>(null);

  useEffect(() => {
    if (query.data) {
      setText(JSON.stringify(query.data, null, 2));
      setJsonError(null);
    }
  }, [query.data]);

  const save = useMutation({
    mutationFn: (value: Record<string, unknown>) => commonConfigApi.put('claude-code', value),
    onSuccess: () => {
      setJsonError(null);
      onSaved('Common Config 已保存；下次切换或显式应用后生效');
      void queryClient.invalidateQueries({ queryKey: ['common-config', 'claude-code'] });
    },
    onError: (e: Error) => {
      setJsonError(`保存失败: ${e.message}`);
    },
  });

  function handleSave() {
    const parsed = parseJsonObjectText(text, 'Common Config');
    if (parsed.error || parsed.value === null) {
      setJsonError(parsed.error ?? 'Common Config 必须是 JSON 对象');
      return;
    }
    setJsonError(null);
    save.mutate(parsed.value);
  }

  return (
    <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-5 space-y-3">
      <div className="flex items-start justify-between gap-3">
        <div>
          <h2 className="font-semibold">Claude Code Common Config</h2>
          <p className="text-xs text-gray-500 mt-0.5">
            全局 JSON object 片段，可写 permissions、hooks、statusLine、outputStyle、env 等顶层键。
            保存只更新 DB，下次切换 provider 或显式应用到 live 后生效。
          </p>
        </div>
        <button
          type="button"
          onClick={handleSave}
          disabled={query.isLoading || !!query.error || save.isPending}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
        >
          {save.isPending ? '保存中...' : '保存'}
        </button>
      </div>

      {query.isLoading && <p className="text-sm text-gray-500">加载 Common Config 中...</p>}
      {query.error && <p className="text-sm text-red-500">加载失败: {query.error.message}</p>}

      {!query.isLoading && !query.error && (
        <textarea
          value={text}
          onChange={(e) => {
            setText(e.target.value);
            setJsonError(null);
          }}
          rows={8}
          className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-xs bg-transparent font-mono"
          placeholder='{"includeCoAuthoredBy": false}'
        />
      )}

      {jsonError && <p className="text-xs text-red-500">{jsonError}</p>}
      <p className="text-xs text-gray-500">
        非 object JSON（数组、字符串、数字、null）不会保存；连接层 base_url/token 仍由端点体系注入。
      </p>
    </div>
  );
}

function BannerView({ banner, onDismiss }: { banner: Banner; onDismiss: () => void }) {
  const styles: Record<BannerKind, string> = {
    success: 'bg-green-50 dark:bg-green-900/20 border-green-300 dark:border-green-700 text-green-700 dark:text-green-300',
    warning: 'bg-yellow-50 dark:bg-yellow-900/20 border-yellow-300 dark:border-yellow-700 text-yellow-700 dark:text-yellow-300',
    error: 'bg-red-50 dark:bg-red-900/20 border-red-300 dark:border-red-700 text-red-700 dark:text-red-300',
  };
  return (
    <div className={`flex items-center justify-between gap-3 px-4 py-2 rounded-md border text-sm ${styles[banner.kind]}`}>
      <span>{banner.text}</span>
      <button
        type="button"
        onClick={onDismiss}
        className="text-xs opacity-70 hover:opacity-100"
        aria-label="关闭提示"
      >
        ✕
      </button>
    </div>
  );
}
