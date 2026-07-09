import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query';
import { useState } from 'react';
import {
  skillsApi,
  type ImportSkillDirBody,
  type Skill,
  type SkillApp,
  type SkillSyncReport,
} from '../lib/api';

const APP_FIELDS: { app: SkillApp; field: keyof Pick<Skill, 'enabled_claude' | 'enabled_codex' | 'enabled_gemini' | 'enabled_opencode' | 'enabled_hermes'>; label: string }[] = [
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
  const [banner, setBanner] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null);

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
      setBanner({ kind: reports.some((r) => r.conflicts.length) ? 'err' : 'ok', text: summarizeReports(reports) });
    },
    onError: (e: Error) => setBanner({ kind: 'err', text: `同步失败：${e.message}` }),
  });

  return (
    <div className="space-y-6">
      <div className="flex items-start justify-between gap-4">
        <div>
          <h1 className="text-2xl font-bold">Skills</h1>
          <p className="text-sm text-gray-500 mt-1">
            管理本地 Skill 清单。当前版本支持本地目录导入、SSOT 复制保存、按 app 启用并 copy 投影；网络发现和更新稍后接入。
          </p>
        </div>
        <button
          onClick={() => sync.mutate()}
          disabled={sync.isPending}
          className="px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm hover:bg-gray-50 dark:hover:bg-gray-800 disabled:opacity-50"
        >
          {sync.isPending ? '同步中...' : '同步全部'}
        </button>
      </div>

      <div className="bg-amber-50 dark:bg-amber-900/20 border border-amber-200 dark:border-amber-800 rounded-md p-3 text-xs text-amber-700 dark:text-amber-300 space-y-1">
        <p className="font-medium">安全边界</p>
        <ul className="list-disc list-inside space-y-0.5">
          <li>导入目录必须包含 <span className="font-mono">SKILL.md</span>，且拒绝符号链接。</li>
          <li>目标同名目录若不是 agent-switch 托管项，会报告冲突并拒绝覆盖。</li>
          <li>目标工具配置目录不存在时只保存启用状态，不凭空创建根目录。</li>
        </ul>
      </div>

      {status && (
        <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 space-y-3">
          <div className="text-xs text-gray-500">
            SSOT：<span className="font-mono">{status.ssot_path}</span>（{status.ssot_exists ? '已创建' : '尚未创建'}）
          </div>
          <div className="grid grid-cols-1 md:grid-cols-2 lg:grid-cols-3 gap-2 text-xs">
            {status.apps.map((app) => (
              <div key={app.app} className="rounded border border-gray-200 dark:border-gray-800 p-2">
                <p className="font-medium">{app.label}</p>
                <p className="text-gray-500 font-mono break-all">{app.target_root}</p>
                <p className="text-gray-500">
                  已启用 {app.enabled_count}，托管投影 {app.managed_count}，{app.config_root_exists ? '配置根存在' : '配置根不存在'}
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

      <ImportDirForm
        onImported={(msg) => {
          invalidate();
          setBanner({ kind: 'ok', text: msg });
        }}
      />

      {isLoading && <p className="text-gray-500">加载中...</p>}
      {error && <p className="text-red-500">加载失败: {(error as Error).message}</p>}

      {skills && skills.length === 0 && (
        <p className="text-gray-500 text-sm">还没有 Skill。先从本地目录导入一个包含 SKILL.md 的目录。</p>
      )}

      {skills && skills.length > 0 && (
        <div className="space-y-2">
          {skills.map((skill) => (
            <div key={skill.id} className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 space-y-3">
              <div className="flex items-start justify-between gap-4">
                <div className="min-w-0">
                  <div className="flex items-center gap-2">
                    <span className="font-medium">{skill.name}</span>
                    <span className="text-xs px-1.5 py-0.5 rounded bg-gray-100 dark:bg-gray-800 text-gray-600 dark:text-gray-300">
                      {skill.directory}
                    </span>
                  </div>
                  {skill.description && <p className="text-xs text-gray-500 mt-1">{skill.description}</p>}
                  <p className="text-xs text-gray-400 mt-1 font-mono break-all">hash {skill.content_hash.slice(0, 16)}...</p>
                </div>
              </div>
              <div className="flex flex-wrap gap-2">
                {APP_FIELDS.map((item) => {
                  const enabled = Boolean(skill[item.field]);
                  return (
                    <button
                      key={item.app}
                      onClick={() => toggle.mutate({ skill, app: item.app, enabled: !enabled })}
                      disabled={toggle.isPending}
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
    <div className="bg-white dark:bg-gray-900 rounded-lg border border-gray-200 dark:border-gray-800 p-4 space-y-3">
      <h2 className="font-semibold">从本地目录导入</h2>
      <div className="grid grid-cols-1 md:grid-cols-2 gap-3">
        <label className="space-y-1 md:col-span-2">
          <span className="text-sm font-medium">源目录路径</span>
          <input
            value={sourcePath}
            onChange={(e) => setSourcePath(e.target.value)}
            placeholder="例如 C:\\Users\\you\\skills\\my-skill"
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-white dark:bg-gray-800"
          />
        </label>
        <label className="space-y-1">
          <span className="text-sm font-medium">目录名（可选）</span>
          <input
            value={directory}
            onChange={(e) => setDirectory(e.target.value)}
            placeholder="默认使用源目录名"
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-white dark:bg-gray-800"
          />
        </label>
        <label className="space-y-1">
          <span className="text-sm font-medium">显示名（可选）</span>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="默认使用目录名"
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-white dark:bg-gray-800"
          />
        </label>
        <label className="space-y-1 md:col-span-2">
          <span className="text-sm font-medium">描述（可选）</span>
          <input
            value={description}
            onChange={(e) => setDescription(e.target.value)}
            className="w-full px-3 py-2 border border-gray-300 dark:border-gray-700 rounded-md text-sm bg-white dark:bg-gray-800"
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
