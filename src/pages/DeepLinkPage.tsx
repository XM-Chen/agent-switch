import { useState } from 'react';
import { DeepLinkImportDialog } from '../components/deeplink/DeepLinkImportDialog';

export function DeepLinkPage() {
  const [open, setOpen] = useState(false);

  return (
    <div className="space-y-6">
      <div>
        <h1 className="text-2xl font-bold">Deep Link 导入</h1>
        <p className="text-sm text-gray-500 mt-1">
          手动粘贴 ccswitch://v1/import 链接进行预览。只有点击确认导入才会写入数据库或同步配置。
        </p>
      </div>

      <div className="rounded-lg border border-gray-200 dark:border-gray-800 bg-white dark:bg-gray-900 p-5 space-y-3">
        <p className="text-sm text-gray-600 dark:text-gray-300">
          支持 provider、prompt、mcp、skill 四类资源。Provider 的 API Key 会走 endpoint 加密保存，Prompt/MCP 会复用现有服务；Skill 当前仅解析预览，等待完整安装服务接入。
        </p>
        <button
          type="button"
          onClick={() => setOpen(true)}
          className="px-4 py-2 bg-blue-600 text-white rounded-md text-sm hover:bg-blue-700"
        >
          粘贴 Deep Link
        </button>
      </div>

      {open && <DeepLinkImportDialog onClose={() => setOpen(false)} />}
    </div>
  );
}
