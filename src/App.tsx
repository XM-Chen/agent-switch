import { useEffect, useState } from 'react';
import { Routes, Route, Navigate } from 'react-router-dom';
import { listen } from '@tauri-apps/api/event';
import { AppShell } from './components/layout/AppShell';
import { DeepLinkImportDialog } from './components/deeplink/DeepLinkImportDialog';
import { DashboardPage } from './pages/DashboardPage';
import { AccountsPage } from './pages/AccountsPage';
import { EndpointsPage } from './pages/EndpointsPage';
import { ModelsPage } from './pages/ModelsPage';
import { ToolsPage } from './pages/ToolsPage';
import { RoutesPage } from './pages/RoutesPage';
import { LogsPage } from './pages/LogsPage';
import { SettingsPage } from './pages/SettingsPage';
import { ProvidersPage } from './pages/ProvidersPage';
import { McpPage } from './pages/McpPage';
import { PromptsPage } from './pages/PromptsPage';
import { SessionsPage } from './pages/SessionsPage';
import { SkillsPage } from './pages/SkillsPage';
import { DeepLinkPage } from './pages/DeepLinkPage';

export default function App() {
  const [deepLinkUrl, setDeepLinkUrl] = useState<string | null>(null);

  useEffect(() => {
    let unlisten: (() => void) | undefined;
    void listen<string>('deeplink-import-url', (event) => {
      setDeepLinkUrl(event.payload);
    })
      .then((fn) => {
        unlisten = fn;
      })
      .catch(() => {});
    return () => unlisten?.();
  }, []);

  return (
    <>
      <AppShell>
        <Routes>
          <Route path="/" element={<DashboardPage />} />
          <Route path="/accounts" element={<AccountsPage />} />
          <Route path="/endpoints" element={<EndpointsPage />} />
          <Route path="/models" element={<ModelsPage />} />
          <Route path="/tools" element={<ToolsPage />} />
          <Route path="/providers" element={<ProvidersPage />} />
          <Route path="/mcp" element={<McpPage />} />
          <Route path="/prompts" element={<PromptsPage />} />
          <Route path="/sessions" element={<SessionsPage />} />
          <Route path="/skills" element={<SkillsPage />} />
          <Route path="/deeplink" element={<DeepLinkPage />} />
          <Route path="/routes" element={<RoutesPage />} />
          <Route path="/logs" element={<LogsPage />} />
          <Route path="/settings" element={<SettingsPage />} />
          <Route path="*" element={<Navigate to="/" replace />} />
        </Routes>
      </AppShell>
      {deepLinkUrl && (
        <DeepLinkImportDialog
          key={deepLinkUrl}
          initialUrl={deepLinkUrl}
          onClose={() => setDeepLinkUrl(null)}
        />
      )}
    </>
  );
}
