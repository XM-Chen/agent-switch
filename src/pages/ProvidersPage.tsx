import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useMemo, useRef, useState } from 'react';
import {
  providersApi,
  type CreateProviderBody,
  type Provider,
  type UpdateProviderBody,
} from '../lib/api';
import { AppTypeSection } from '../components/providers/AppTypeSection';
import { ProviderForm } from '../components/providers/ProviderForm';
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

      {banner && <BannerView banner={banner} onDismiss={() => setBanner(null)} />}

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
        />
      )}
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
