import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import {
  skillsApi,
  type DiscoveredSkill,
  type ImportSkillDirBody,
  type InstallRepoBody,
  type Skill,
  type SkillApp,
  type SkillBackupEntry,
  type SkillSyncReport,
  type SkillUpdateItemReport,
  type UnmanagedSkill,
} from '../lib/api';

const APP_FIELDS: {
  app: SkillApp;
  field: keyof Pick<Skill, 'enabled_claude' | 'enabled_codex' | 'enabled_gemini' | 'enabled_opencode' | 'enabled_hermes'>;
  label: string;
}[] = [
  { app: 'claude', field: 'enabled_claude', label: 'Claude' },
  { app: 'codex', field: 'enabled_codex', label: 'Codex' },
  { app: 'gemini', field: 'enabled_gemini', label: 'Gemini' },
  { app: 'opencode', field: 'enabled_opencode', label: 'OpenCode' },
  { app: 'hermes', field: 'enabled_hermes', label: 'Hermes' },
];

function summarizeReports(reports: SkillSyncReport[]): string {
  if (!reports.length) return '未触发投影。';
  return reports
    .map((r) => {
      const bits = [`${r.label}: 投影 ${r.projected}，移除 ${r.removed}`];
      if (r.skipped_missing_root) bits.push(`目标未安装跳过 ${r.skipped_missing_root}`);
      if (r.conflicts.length) bits.push(`冲突 ${r.conflicts.length}`);
      return bits.join('，');
    })
    .join('\n');
}

const inputClass =
  'w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-white dark:bg-gray-800';
const cardClass =
  'bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 space-y-3';

type Banner = { kind: 'ok' | 'err'; text: string };

export function SkillsPage() {
  const queryClient = useQueryClient();
  const { data: skills, isLoading, error } = useQuery({
    queryKey: ['skills'],
    queryFn: skillsApi.list,
  });
  const { data: status } = useQuery({
    queryKey: ['skills', 'status'],
    queryFn: skillsApi.status,
  });
  const [banner, setBanner] = useState<Banner | null>(null);

  const invalidate = () => {
    queryClient.invalidateQueries({ queryKey: ['skills'] });
  };

  const toggle = useMutation({
    mutationFn: ({ skill, app, enabled }: { skill: Skill; app: SkillApp; enabled: boolean }) =>
      skillsApi.setEnabled(skill.id, app, enabled),
    onSuccess: (report) => {
      invalidate();
      setBanner({ kind: report.conflicts.length ? 'err' : 'ok', text: summarizeReports([report]) });
    },
    onError: (e: Error) => setBanner({ kind: 'err', text: `切换失败：${e.message}` }),
  });

  const sync = useMutation({
    mutationFn: () => skillsApi.sync(),
    onSuccess: (reports) => {
      invalidate();
      setBanner({
        kind: reports.some((r) => r.conflicts.length) ? 'err' : 'ok',
        text: summarizeReports(reports),
      });
    },
    onError: (e: Error) => setBanner({ kind: 'err', text: `同步失败：${e.message}` }),
  });

  const checkUpdates = useMutation({
    mutationFn: () => skillsApi.checkUpdates(),
    onSuccess: (report) => {
      invalidate();
      const available = report.checked.filter((c) => c.status === 'update_available');
      const errs = report.checked.filter((c) => c.status === 'error');
      const parts = [`检查 ${report.checked.length} 个 GitHub 来源 skill`];
      parts.push(available.length ? `可更新 ${available.length} 个` : '均为最新');
      if (errs.length) parts.push(`检查失败 ${errs.length} 个`);
      setBanner({ kind: errs.length ? 'err' : 'ok', text: parts.join('，') });
    },
    onError: (e: Error) => setBanner({ kind: 'err', text: `检查更新失败：${e.message}` }),
  });

  const updateAll = useMutation({
    mutationFn: () => skillsApi.update(),
    onSuccess: (report) => {
      invalidate();
      setBanner(summarizeUpdate(report.items));
    },
    onError: (e: Error) => setBanner({ kind: 'err', text: `批量更新失败：${e.message}` }),
  });

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold">Skills</h1>
          <p className="text-sm text-gray-500 mt-1">
            管理 Skill 清单：本地目录 / zip 导入、GitHub 安装与发现、SSOT 复制保存、按 app 启用并 copy
            投影、卸载备份与更新。网络操作仅在你显式触发时发生。
          </p>
        </div>
        <div className="flex flex-wrap gap-2">
          <button
            onClick={() => checkUpdates.mutate()}
            disabled={checkUpdates.isPending}
            className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50"
          >
            {checkUpdates.isPending ? '检查中...' : '检查更新'}
          </button>
          <button
            onClick={() => {
              if (window.confirm('批量更新会联网拉取所有 GitHub 来源 skill 的最新内容，并在更新前自动备份。是否继续？')) {
                updateAll.mutate();
              }
            }}
            disabled={updateAll.isPending}
            className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50"
          >
            {updateAll.isPending ? '更新中...' : '全部更新'}
          </button>
          <button
            onClick={() => sync.mutate()}
            disabled={sync.isPending}
            className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50"
          >
            {sync.isPending ? '同步中...' : '同步全部'}
          </button>
        </div>
      </div>

      <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-md p-3 text-xs text-amber-700 dark:text-amber-300 space-y-1">
        <p className="font-medium">安全边界</p>
        <ul className="list-disc list-inside space-y-0.5">
          <li>导入/下载的目录必须包含 <span className="font-mono">SKILL.md</span>，且拒绝符号链接与路径穿越。</li>
          <li>目标同名目录若不是 agent-switch 托管项，会报告冲突并拒绝覆盖。</li>
          <li>目标工具配置目录不存在时只保存启用状态，不凭空创建根目录。</li>
          <li>卸载与更新前会自动备份 SSOT 与记录，更新失败会从备份回滚。</li>
        </ul>
      </div>

      {status && (
        <div className={cardClass}>
          <div className="text-xs text-gray-500">
            SSOT：<span className="font-mono">{status.ssot_path}</span>（{status.ssot_exists ? '已创建' : '尚未创建'}）
          </div>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-2 text-xs">
            {status.apps.map((app) => (
              <div key={app.app} className="rounded border border-gray-200 dark:border-gray-800 p-2">
                <p className="font-medium">{app.label}</p>
                <p className="text-gray-500 font-mono break-all">{app.target_root}</p>
                <p className="text-gray-500">
                  已启用 {app.enabled_count}，托管投影 {app.managed_count}，
                  {app.config_root_exists ? '配置根存在' : '配置根不存在'}
                </p>
                {app.conflicts.length > 0 && <p className="text-red-500">冲突 {app.conflicts.length} 项</p>}
              </div>
            ))}
          </div>
        </div>
      )}

      {banner && (
        <div
          className={`rounded-md p-3 text-sm whitespace-pre-wrap ${
            banner.kind === 'ok'
              ? 'bg-green-50 dark:bg-green-900/20 border border-green-200 dark:border-green-800 text-green-700 dark:text-green-300'
              : 'bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 text-red-700 dark:text-red-300'
          }`}
        >
          {banner.text}
        </div>
      )}

      <div className="grid grid-cols-1 lg:grid-cols-2 gap-4">
        <InstallRepoForm onDone={setBanner} onChanged={invalidate} />
        <ImportZipForm onDone={setBanner} onChanged={invalidate} />
      </div>

      <SearchPanel onDone={setBanner} onChanged={invalidate} />

      <ScanUnmanagedPanel onDone={setBanner} />

      <ImportDirForm
        onImported={(msg) => {
          invalidate();
          setBanner({ kind: 'ok', text: msg });
        }}
      />

      {isLoading && <p className="text-gray-500">加载中...</p>}
      {error && <p className="text-red-500">加载失败: {(error as Error).message}</p>}

      {skills && skills.length === 0 && (
        <p className="text-gray-500 text-sm">还没有 Skill。从本地目录 / zip 导入，或从 GitHub 安装一个包含 SKILL.md 的目录。</p>
      )}

      {skills && skills.length > 0 && (
        <div className="space-y-2">
          {skills.map((skill) => (
            <SkillCard
              key={skill.id}
              skill={skill}
              onToggle={(app, enabled) => toggle.mutate({ skill, app, enabled })}
              toggling={toggle.isPending}
              onDone={setBanner}
              onChanged={invalidate}
            />
          ))}
        </div>
      )}
    </div>
  );
}

function summarizeUpdate(items: SkillUpdateItemReport[]): Banner {
  const updated = items.filter((i) => i.updated);
  const failed = items.filter((i) => i.error);
  const parts = [`处理 ${items.length} 个`];
  parts.push(updated.length ? `已更新 ${updated.length} 个` : '无需更新');
  if (failed.length) parts.push(`失败 ${failed.length} 个`);
  const detail = failed.length
    ? '\n' + failed.map((f) => `${f.directory}: ${f.error}`).join('\n')
    : '';
  return { kind: failed.length ? 'err' : 'ok', text: parts.join('，') + detail };
}

function SkillCard({
  skill,
  onToggle,
  toggling,
  onDone,
  onChanged,
}: {
  skill: Skill;
  onToggle: (app: SkillApp, enabled: boolean) => void;
  toggling: boolean;
  onDone: (b: Banner) => void;
  onChanged: () => void;
}) {
  const isGithub = skill.source_type === 'github';

  const updateOne = useMutation({
    mutationFn: () => skillsApi.update([skill.id]),
    onSuccess: (report) => {
      onChanged();
      onDone(summarizeUpdate(report.items));
    },
    onError: (e: Error) => onDone({ kind: 'err', text: `更新失败：${e.message}` }),
  });

  const uninstall = useMutation({
    mutationFn: () => skillsApi.uninstall(skill.id),
    onSuccess: (report) => {
      onChanged();
      onDone({
        kind: 'ok',
        text: `已卸载 ${report.directory}，备份时间戳 ${report.backup.timestamp}。可从下方备份恢复。`,
      });
    },
    onError: (e: Error) => onDone({ kind: 'err', text: `卸载失败：${e.message}` }),
  });

  return (
    <div className={cardClass}>
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2 flex-wrap">
            <span className="font-medium">{skill.name}</span>
            <span className="text-xs px-1.5 py-0.5 rounded bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-300">
              {skill.directory}
            </span>
            <span className="text-xs px-1.5 py-0.5 rounded bg-blue-50 dark:bg-blue-900/20 text-blue-600 dark:text-blue-300">
              {skill.source_type}
            </span>
          </div>
          {skill.description && <p className="text-xs text-gray-500 mt-1">{skill.description}</p>}
          {isGithub && skill.repo_owner && skill.repo_name && (
            <p className="text-xs text-gray-400 mt-1 font-mono break-all">
              {skill.repo_owner}/{skill.repo_name}
              {skill.repo_subdir ? ` · ${skill.repo_subdir}` : ''}
              {skill.repo_branch ? ` @${skill.repo_branch}` : ''}
            </p>
          )}
          <p className="text-xs text-gray-400 mt-1 font-mono break-all">
            hash {skill.content_hash.slice(0, 16)}...
          </p>
        </div>
        <div className="flex gap-2 shrink-0">
          {isGithub && (
            <button
              onClick={() => {
                if (window.confirm(`联网拉取 ${skill.directory} 的最新内容并替换 SSOT（更新前自动备份）。是否继续？`)) {
                  updateOne.mutate();
                }
              }}
              disabled={updateOne.isPending}
              className="px-3 py-1.5 rounded-md border border-gray-300 dark:border-gray-700 text-xs hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50"
            >
              {updateOne.isPending ? '更新中...' : '更新'}
            </button>
          )}
          <button
            onClick={() => {
              if (window.confirm(`卸载 ${skill.directory} 会删除 SSOT、DB 记录与已投影的 live 目录（卸载前自动备份）。是否继续？`)) {
                uninstall.mutate();
              }
            }}
            disabled={uninstall.isPending}
            className="px-3 py-1.5 rounded-md border border-red-300 dark:border-red-800 text-xs text-red-600 dark:text-red-300 hover:bg-red-50 dark:hover:bg-red-900/20 disabled:opacity-50"
          >
            {uninstall.isPending ? '卸载中...' : '卸载'}
          </button>
        </div>
      </div>
      <div className="flex flex-wrap gap-2">
        {APP_FIELDS.map((item) => {
          const enabled = Boolean(skill[item.field]);
          return (
            <button
              key={item.app}
              onClick={() => onToggle(item.app, !enabled)}
              disabled={toggling}
              className={`px-3 py-1.5 rounded-md border text-xs disabled:opacity-50 ${
                enabled
                  ? 'bg-green-50 dark:bg-green-900/20 border-green-300 dark:border-green-800 text-green-700 dark:text-green-300'
                  : 'border-gray-300 dark:border-gray-700 text-gray-600 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-800'
              }`}
            >
              {item.label} {enabled ? '已启用' : '未启用'}
            </button>
          );
        })}
      </div>
      <BackupsRow directory={skill.directory} onDone={onDone} onChanged={onChanged} />
    </div>
  );
}

function BackupsRow({
  directory,
  onDone,
  onChanged,
}: {
  directory: string;
  onDone: (b: Banner) => void;
  onChanged: () => void;
}) {
  const [open, setOpen] = useState(false);
  const backups = useQuery({
    queryKey: ['skills', 'backups', directory],
    queryFn: () => skillsApi.listBackups(directory),
    enabled: open,
  });

  const restore = useMutation({
    mutationFn: (b: SkillBackupEntry) => skillsApi.restore(b.directory, b.timestamp),
    onSuccess: (report) => {
      onChanged();
      onDone({ kind: 'ok', text: `已从备份恢复 ${report.directory}。\n${summarizeReports(report.sync)}` });
    },
    onError: (e: Error) => onDone({ kind: 'err', text: `恢复失败：${e.message}` }),
  });

  return (
    <div className="pt-2 border-t border-gray-100 dark:border-gray-800">
      <button
        onClick={() => setOpen((v) => !v)}
        className="text-xs text-gray-500 hover:text-gray-700 dark:hover:text-gray-300"
      >
        {open ? '收起备份' : '查看备份'}
      </button>
      {open && (
        <div className="mt-2 space-y-1">
          {backups.isLoading && <p className="text-xs text-gray-400">加载备份中...</p>}
          {backups.data && backups.data.length === 0 && (
            <p className="text-xs text-gray-400">暂无备份。</p>
          )}
          {backups.data?.map((b) => (
            <div key={b.timestamp} className="flex items-center justify-between gap-2 text-xs">
              <span className="font-mono text-gray-500">
                {new Date(Number(b.timestamp)).toLocaleString()}（{b.timestamp}）
              </span>
              <button
                onClick={() => {
                  if (window.confirm(`用该备份覆盖当前 ${b.directory} 的 SSOT 内容与记录。是否继续？`)) {
                    restore.mutate(b);
                  }
                }}
                disabled={restore.isPending || !b.has_snapshot}
                className="px-2 py-1 rounded border border-gray-300 dark:border-gray-700 hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50"
              >
                恢复
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function InstallRepoForm({ onDone, onChanged }: { onDone: (b: Banner) => void; onChanged: () => void }) {
  const [repo, setRepo] = useState('');
  const [branch, setBranch] = useState('');
  const [subdir, setSubdir] = useState('');
  const [directory, setDirectory] = useState('');
  const [localError, setLocalError] = useState<string | null>(null);

  const install = useMutation({
    mutationFn: (body: InstallRepoBody) => skillsApi.installRepo(body),
    onSuccess: (report) => {
      setRepo('');
      setBranch('');
      setSubdir('');
      setDirectory('');
      onChanged();
      onDone({ kind: 'ok', text: `已从 GitHub 安装 ${report.skill.name}。\n${summarizeReports(report.sync)}` });
    },
    onError: (e: Error) => setLocalError(e.message),
  });

  const submit = () => {
    setLocalError(null);
    if (!repo.trim()) {
      setLocalError('repo 不能为空（格式 owner/name）。');
      return;
    }
    install.mutate({
      repo: repo.trim(),
      branch: branch.trim() || null,
      subdir: subdir.trim() || null,
      directory: directory.trim() || null,
    });
  };

  return (
    <div className={cardClass}>
      <h2 className="font-semibold">从 GitHub 安装</h2>
      <label className="space-y-1 block">
        <span className="text-sm font-medium">仓库（owner/name）</span>
        <input value={repo} onChange={(e) => setRepo(e.target.value)} placeholder="anthropics/skills" className={inputClass} />
      </label>
      <div className="grid grid-cols-1 md:grid-cols-3 gap-3">
        <label className="space-y-1">
          <span className="text-sm font-medium">分支（可选）</span>
          <input value={branch} onChange={(e) => setBranch(e.target.value)} placeholder="默认主分支" className={inputClass} />
        </label>
        <label className="space-y-1">
          <span className="text-sm font-medium">子目录（可选）</span>
          <input value={subdir} onChange={(e) => setSubdir(e.target.value)} placeholder="如 skills/foo" className={inputClass} />
        </label>
        <label className="space-y-1">
          <span className="text-sm font-medium">目录名（可选）</span>
          <input value={directory} onChange={(e) => setDirectory(e.target.value)} placeholder="默认推导" className={inputClass} />
        </label>
      </div>
      {localError && <p className="text-sm text-red-600 dark:text-red-300 whitespace-pre-wrap">{localError}</p>}
      <div className="flex justify-end">
        <button
          onClick={submit}
          disabled={install.isPending}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
        >
          {install.isPending ? '安装中...' : '联网安装'}
        </button>
      </div>
    </div>
  );
}

function ImportZipForm({ onDone, onChanged }: { onDone: (b: Banner) => void; onChanged: () => void }) {
  const [zipPath, setZipPath] = useState('');
  const [subdir, setSubdir] = useState('');
  const [directory, setDirectory] = useState('');
  const [localError, setLocalError] = useState<string | null>(null);

  const importZip = useMutation({
    mutationFn: () =>
      skillsApi.importZip({
        zip_path: zipPath.trim(),
        subdir: subdir.trim() || null,
        directory: directory.trim() || null,
      }),
    onSuccess: (report) => {
      setZipPath('');
      setSubdir('');
      setDirectory('');
      onChanged();
      onDone({ kind: 'ok', text: `已从 zip 导入 ${report.skill.name}。\n${summarizeReports(report.sync)}` });
    },
    onError: (e: Error) => setLocalError(e.message),
  });

  const submit = () => {
    setLocalError(null);
    if (!zipPath.trim()) {
      setLocalError('zip 路径不能为空。');
      return;
    }
    importZip.mutate();
  };

  return (
    <div className={cardClass}>
      <h2 className="font-semibold">从 zip 导入</h2>
      <label className="space-y-1 block">
        <span className="text-sm font-medium">zip 文件路径</span>
        <input value={zipPath} onChange={(e) => setZipPath(e.target.value)} placeholder="C:\\path\\to\\skill.zip" className={inputClass} />
      </label>
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        <label className="space-y-1">
          <span className="text-sm font-medium">子目录（可选）</span>
          <input value={subdir} onChange={(e) => setSubdir(e.target.value)} placeholder="zip 内 skill 子目录" className={inputClass} />
        </label>
        <label className="space-y-1">
          <span className="text-sm font-medium">目录名（可选）</span>
          <input value={directory} onChange={(e) => setDirectory(e.target.value)} placeholder="默认用文件名" className={inputClass} />
        </label>
      </div>
      {localError && <p className="text-sm text-red-600 dark:text-red-300 whitespace-pre-wrap">{localError}</p>}
      <div className="flex justify-end">
        <button
          onClick={submit}
          disabled={importZip.isPending}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
        >
          {importZip.isPending ? '导入中...' : '导入 zip'}
        </button>
      </div>
    </div>
  );
}

function SearchPanel({ onDone, onChanged }: { onDone: (b: Banner) => void; onChanged: () => void }) {
  const [query, setQuery] = useState('');
  const [results, setResults] = useState<DiscoveredSkill[] | null>(null);

  const search = useMutation({
    mutationFn: () => skillsApi.search(query.trim()),
    onSuccess: (report) => {
      setResults(report.results);
      if (!report.results.length) onDone({ kind: 'ok', text: `未找到与「${report.query}」相关的候选。` });
    },
    onError: (e: Error) => onDone({ kind: 'err', text: `搜索失败：${e.message}` }),
  });

  const install = useMutation({
    mutationFn: (d: DiscoveredSkill) =>
      skillsApi.installRepo({ repo: d.repo, branch: d.default_branch ?? null }),
    onSuccess: (report) => {
      onChanged();
      onDone({ kind: 'ok', text: `已安装 ${report.skill.name}。\n${summarizeReports(report.sync)}` });
    },
    onError: (e: Error) => onDone({ kind: 'err', text: `安装失败：${e.message}` }),
  });

  return (
    <div className={cardClass}>
      <h2 className="font-semibold">GitHub 发现</h2>
      <div className="flex gap-2">
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter' && query.trim()) search.mutate();
          }}
          placeholder="搜索关键字，如 code review"
          className={inputClass}
        />
        <button
          onClick={() => query.trim() && search.mutate()}
          disabled={search.isPending || !query.trim()}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50 shrink-0"
        >
          {search.isPending ? '搜索中...' : '搜索'}
        </button>
      </div>
      {results && results.length > 0 && (
        <div className="space-y-2">
          {results.map((d) => (
            <div key={d.repo} className="flex items-start justify-between gap-3 rounded border border-gray-200 dark:border-gray-800 p-2">
              <div className="min-w-0">
                <div className="flex items-center gap-2 flex-wrap">
                  <a href={d.html_url} target="_blank" rel="noreferrer" className="text-sm font-medium text-blue-600 dark:text-blue-400 hover:underline break-all">
                    {d.repo}
                  </a>
                  <span className="text-xs text-gray-400">★ {d.stars}</span>
                </div>
                {d.description && <p className="text-xs text-gray-500 mt-0.5 line-clamp-2">{d.description}</p>}
              </div>
              <button
                onClick={() => {
                  if (window.confirm(`从 ${d.repo} 联网安装 skill？安装内容来自外部仓库，请确认可信。`)) {
                    install.mutate(d);
                  }
                }}
                disabled={install.isPending}
                className="px-3 py-1.5 rounded-md border border-gray-300 dark:border-gray-700 text-xs hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50 shrink-0"
              >
                安装
              </button>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ScanUnmanagedPanel({ onDone }: { onDone: (b: Banner) => void }) {
  const [items, setItems] = useState<UnmanagedSkill[] | null>(null);

  const scan = useMutation({
    mutationFn: () => skillsApi.scanUnmanaged(),
    onSuccess: (report) => {
      setItems(report.items);
      onDone({ kind: 'ok', text: `扫描到 ${report.items.length} 个非托管目录。` });
    },
    onError: (e: Error) => onDone({ kind: 'err', text: `扫描失败：${e.message}` }),
  });

  return (
    <div className={cardClass}>
      <div className="flex items-center justify-between gap-4">
        <div>
          <h2 className="font-semibold">扫描未托管 Skill</h2>
          <p className="text-xs text-gray-500 mt-1">
            只读扫描各工具 live skills 目录中「非 agent-switch 托管」的目录，供你判断是否手动导入纳管。不会改动 live 目录。
          </p>
        </div>
        <button
          onClick={() => scan.mutate()}
          disabled={scan.isPending}
          className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50 shrink-0"
        >
          {scan.isPending ? '扫描中...' : '扫描'}
        </button>
      </div>
      {items && items.length === 0 && <p className="text-xs text-gray-400">未发现非托管目录。</p>}
      {items && items.length > 0 && (
        <div className="space-y-1">
          {items.map((it) => (
            <div key={`${it.app}-${it.directory}`} className="flex items-center justify-between gap-2 text-xs rounded border border-gray-200 dark:border-gray-800 p-2">
              <div className="min-w-0">
                <span className="font-medium">{it.label}</span> ·{' '}
                <span className="font-mono">{it.directory}</span>
                <span className="text-gray-400 ml-2">{it.has_entry_file ? '含 SKILL.md' : '缺 SKILL.md'}</span>
                {it.known_directory && <span className="text-amber-500 ml-2">目录名已在清单中</span>}
              </div>
              <span className="font-mono text-gray-400 break-all">{it.path}</span>
            </div>
          ))}
        </div>
      )}
    </div>
  );
}

function ImportDirForm({ onImported }: { onImported: (msg: string) => void }) {
  const [sourcePath, setSourcePath] = useState('');
  const [directory, setDirectory] = useState('');
  const [name, setName] = useState('');
  const [description, setDescription] = useState('');
  const [enabledClaude, setEnabledClaude] = useState(true);
  const [localError, setLocalError] = useState<string | null>(null);

  const importDir = useMutation({
    mutationFn: (body: ImportSkillDirBody) => skillsApi.importDir(body),
    onSuccess: (report) => {
      setSourcePath('');
      setDirectory('');
      setName('');
      setDescription('');
      onImported(`已导入 ${report.skill.name}。\n${summarizeReports(report.sync)}`);
    },
    onError: (e: Error) => setLocalError(e.message),
  });

  const submit = () => {
    setLocalError(null);
    if (!sourcePath.trim()) {
      setLocalError('源目录不能为空。');
      return;
    }
    importDir.mutate({
      source_path: sourcePath.trim(),
      directory: directory.trim() || null,
      name: name.trim() || null,
      description: description.trim() || null,
      enabled_claude: enabledClaude,
    });
  };

  return (
    <div className={cardClass}>
      <h2 className="font-semibold">从本地目录导入</h2>
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        <label className="space-y-1 md:col-span-2">
          <span className="text-sm font-medium">源目录路径</span>
          <input
            value={sourcePath}
            onChange={(e) => setSourcePath(e.target.value)}
            placeholder="例如 C:\\Users\\you\\skills\\my-skill"
            className={inputClass}
          />
        </label>
        <label className="space-y-1">
          <span className="text-sm font-medium">目录名（可选）</span>
          <input
            value={directory}
            onChange={(e) => setDirectory(e.target.value)}
            placeholder="默认使用源目录名"
            className={inputClass}
          />
        </label>
        <label className="space-y-1">
          <span className="text-sm font-medium">显示名（可选）</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="默认使用目录名"
            className={inputClass}
          />
        </label>
        <label className="space-y-1 md:col-span-2">
          <span className="text-sm font-medium">描述（可选）</span>
          <input
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            className={inputClass}
          />
        </label>
      </div>
      <label className="flex items-center gap-2 text-sm">
        <input type="checkbox" checked={enabledClaude} onChange={(e) => setEnabledClaude(e.target.checked)} />
        导入后启用到 Claude Code（若 ~/.claude 存在则立即 copy 投影）
      </label>
      {localError && (
        <div className="bg-red-50 dark:bg-red-900/20 border border-red-200 dark:border-red-800 rounded-md p-3 text-sm text-red-700 dark:text-red-300 whitespace-pre-wrap">
          {localError}
        </div>
      )}
      <div className="flex justify-end">
        <button
          onClick={submit}
          disabled={importDir.isPending}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700 disabled:opacity-50"
        >
          {importDir.isPending ? '导入中...' : '导入目录'}
        </button>
      </div>
    </div>
  );
}
