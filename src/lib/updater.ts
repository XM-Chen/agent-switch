import { check, type DownloadEvent } from '@tauri-apps/plugin-updater';
import { relaunch } from '@tauri-apps/plugin-process';
import { getVersion } from '@tauri-apps/api/app';

/// 下载进度回调参数：已下载字节数 / 总字节数（总数可能为 undefined）。
export interface DownloadProgress {
  downloaded: number;
  contentLength?: number;
}

/// 检查更新结果。available=false 时其余字段为空。
export interface UpdateInfo {
  available: boolean;
  version?: string;
  notes?: string;
  /// 有新版时提供：下载 + 校验签名 + 安装 + 重启。onProgress 用于 UI 展示进度。
  downloadAndInstall?: (onProgress?: (p: DownloadProgress) => void) => Promise<void>;
}

/// 读取当前应用版本号（来自 tauri.conf.json 的 version）。
export async function getCurrentVersion(): Promise<string> {
  return getVersion();
}

/// 检查是否有可用更新。
/// 内部：GET endpoints[0] → 拉 latest.json → 比对版本 → 返回 Update 或 null。
/// 无更新返回 { available: false }；有更新返回携带 downloadAndInstall 的 UpdateInfo。
/// 网络失败 / 404 / 解析失败等错误向上抛出，由调用方转成中文提示。
export async function checkForUpdate(): Promise<UpdateInfo> {
  const update = await check();
  if (!update) {
    return { available: false };
  }

  return {
    available: true,
    version: update.version,
    notes: update.body,
    downloadAndInstall: async (onProgress?: (p: DownloadProgress) => void) => {
      let downloaded = 0;
      let contentLength: number | undefined;

      // 插件内部：下载 url → 用 tauri.conf.json 的 pubkey 校验 signature → 安装。
      // 校验失败 / 下载损坏会 reject，不会半安装。
      await update.downloadAndInstall((event: DownloadEvent) => {
        switch (event.event) {
          case 'Started':
            contentLength = event.data.contentLength;
            downloaded = 0;
            onProgress?.({ downloaded, contentLength });
            break;
          case 'Progress':
            downloaded += event.data.chunkLength;
            onProgress?.({ downloaded, contentLength });
            break;
          case 'Finished':
            onProgress?.({ downloaded: contentLength ?? downloaded, contentLength });
            break;
        }
      });

      // 安装完成后重启使新版本生效（依赖 tauri-plugin-process）。
      await relaunch();
    },
  };
}
