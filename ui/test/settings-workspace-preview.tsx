/** Development-only settings harness. All requests and native effects stay in this document. */
import React, { useEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { ConfigProvider } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import { HashRouter, Routes, Route, useNavigate } from 'react-router-dom';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { createInstance } from 'i18next';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/modal-contract.css';
import zh from '../src/renderer/services/i18n/locales/zh-CN';
import en from '../src/renderer/services/i18n/locales/en-US';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '../src/renderer/pages/settings/DisplaySettings/presets';
import type { IApiSshConfigScan } from '../src/common/adapter/ipcBridge';

const config: Record<string, unknown> = { language: 'zh-CN', languageMode: 'manual', 'system.keepAwake': true, 'chat.thinking.visible': true };
const response = (data: unknown, status = 200) => new Response(JSON.stringify({ success: status < 400, data }), { status, headers: { 'Content-Type': 'application/json' } });
const permissions = {
  platform: 'windows', app_label: 'NomiFun', permissions: [
    { kind: 'microphone', state: 'granted', can_request: false, can_open_settings: true, requires_restart_after_grant: false, capabilities: ['voice_input'] },
    { kind: 'accessibility', state: 'not_required', can_request: false, can_open_settings: false, requires_restart_after_grant: false, capabilities: ['computer_use'] },
    { kind: 'screen_recording', state: 'not_required', can_request: false, can_open_settings: false, requires_restart_after_grant: false, capabilities: ['computer_use'] },
  ],
};
let hosts = [
  { sshHostId: '0190f5fe-7c00-7a00-8000-000000000201', name: 'Development', host: 'dev.example.test', port: 22, username: 'developer', authType: 'key', privateKey: '***' },
  { sshHostId: '0190f5fe-7c00-7a00-8000-000000000202', name: 'Staging', host: 'staging.example.test', port: 2222, username: 'deploy', authType: 'agent' },
];
// Unrecognized calls fail. No request falls through to a real backend.
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const path = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.origin).pathname;
  const body = typeof init?.body === 'string' ? JSON.parse(init.body) : {};
  if (path === '/api/settings/client') {
    if (init?.method === 'PUT') Object.assign(config, body);
    return response(config);
  }
  if (path === '/api/system/info') return response({ work_dir: 'D:\\NomiFun\\workspace', cache_dir: 'D:\\NomiFun\\cache', log_dir: 'D:\\NomiFun\\logs', storage_generation: 'preview', agent_data_generation: 1, platform: 'windows', arch: 'x86_64' });
  if (path === '/api/system/permissions') return response(permissions);
  if (path === '/api/system/permissions/request') return response(permissions);
  if (path === '/api/system/permissions/open-settings' || path === '/api/system/keep-awake') return response(null);
  if (path === '/api/agent-runtime') return response({ family_id: 'nomifun.nomi', display_name: 'Nomi Runtime', build_id: 'nomi-0.8.2-windows-x64', build_digest: 'a'.repeat(64), host_contract_version: 1, supported_profiles: ['default'] });
  if (path === '/health') return response({ version: '0.8.2' });
  if (path === '/api/ssh-hosts/import-candidates') return response({ configPath: '~/.ssh/config', hosts: [], skippedProxy: [], skippedIncludes: 0 } satisfies IApiSshConfigScan);
  if (path === '/api/providers') return response([]);
  if (path === '/api/plugins') return response({ plugins: [] });
  if (path === '/api/plugins/library-state') return response({ items: [] });
  if (path === '/api/ssh-hosts') return response(hosts);
  if (init?.method === 'DELETE' && path.startsWith('/api/ssh-hosts/')) { hosts = hosts.filter((host) => !path.endsWith(host.sshHostId)); return response(null); }
  return response('Unsupported preview operation', 404);
}) as typeof fetch;

const { ipcBridge } = await import('../src/common');
const { configService } = await import('../src/common/config/configService');
// Native providers are replaced only here; the product's desktop checks stay intact.
ipcBridge.application.getStartOnBootStatus.invoke = async () => ({ success: true, data: { supported: true, enabled: false, isPackaged: true, platform: 'windows' } });
ipcBridge.application.setStartOnBoot.invoke = async ({ enabled }) => ({ success: true, data: { supported: true, enabled, isPackaged: true, platform: 'windows' } });
ipcBridge.application.applyKeepAwake.invoke = async () => {};
ipcBridge.application.setZoomFactor.invoke = async ({ factor }) => factor;
ipcBridge.notification.permissionState.invoke = async () => 'granted';
ipcBridge.notification.requestPermission.invoke = async () => 'granted';
ipcBridge.shell.openFolderWith.invoke = async () => {};
ipcBridge.dialog.showOpen.invoke = async () => [];
ipcBridge.application.restart.invoke = async () => {};
ipcBridge.systemSettings.languageChanged.on = () => () => {};
ipcBridge.mode.onProvidersChanged.on = () => () => {};
ipcBridge.conversation.reconnected.on = () => () => {};
await configService.initialize();
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: zh }, 'en-US': { translation: en } }, interpolation: { escapeValue: false } });
const { default: SystemSettings } = await import('../src/renderer/pages/settings/SystemSettings');
const { default: Sider } = await import('../src/renderer/components/layout/Sider');
const { default: ModelHubPage } = await import('../src/renderer/pages/modelHub');
const { default: HubPageShell } = await import('../src/renderer/components/layout/HubPageShell');
const { AuthProvider } = await import('../src/renderer/hooks/context/AuthContext');
const { default: ExecutionEngineSettings } = await import('../src/renderer/pages/settings/ExecutionEngines');
const { default: SshHostSettings } = await import('../src/renderer/pages/settings/SshHostSettings');
const { ThemeProvider, useThemeContext } = await import('../src/renderer/hooks/context/ThemeContext');

function Preview() {
  const navigate = useNavigate();
  const { theme: mode, setTheme: setMode } = useThemeContext();
  const dark = mode === 'dark';
  const [minimum, setMinimum] = useState(false);
  const [collapsed, setCollapsed] = useState(false);
  const [theme, setTheme] = useState(DEFAULT_THEME_ID);
  useEffect(() => { themeStyle.textContent = PRESET_THEMES.find((item) => item.id === theme)?.css ?? ''; }, [theme]);
  return <>
    <div className='settings-preview-tools'>
      <span>设置交互预览 · 测试数据</span>
      <button onClick={() => navigate('/guid')}>首页导航</button>
      <button onClick={() => navigate('/models')}>模型管理</button>
      <button onClick={() => setCollapsed(!collapsed)}>{collapsed ? '展开侧栏' : '收起侧栏'}</button>
      <button onClick={() => void setMode(dark ? 'light' : 'dark')}>{dark ? '浅色' : '深色'}</button>
      <button onClick={() => setMinimum(!minimum)}>{minimum ? '完整窗口' : '880 × 600'}</button>
      <button onClick={() => void i18n.changeLanguage(i18n.language === 'zh-CN' ? 'en-US' : 'zh-CN')}>中 / EN</button>
      <select aria-label='预览主题' value={theme} onChange={(event) => setTheme(event.target.value)}>
        {PRESET_THEMES.map((item) => <option key={item.id} value={item.id}>{item.name}</option>)}
      </select>
    </div>
    <div className={'settings-preview-frame' + (minimum ? ' settings-preview-frame--minimum' : '') + (collapsed ? ' settings-preview-frame--collapsed' : '')}>
      <aside className='layout-sider'><AuthProvider><Sider collapsed={collapsed} /></AuthProvider></aside>
      <main><Routes>
        <Route path='/settings/system' element={<SystemSettings />} />
        <Route path='/settings/permissions' element={<SystemSettings />} />
        <Route path='/settings/about' element={<SystemSettings />} />
        <Route path='/settings/execution-engines' element={<ExecutionEngineSettings />} />
        <Route path='/settings/ssh-hosts' element={<SshHostSettings />} />
        <Route path='/models' element={<ModelHubPage />} />
        <Route path='*' element={<HubPageShell title='全局导航预览' subtitle='点击左侧设置可返回实际设置页面；此处仅用于检查首页侧栏。'><div className='text-13px text-t-secondary'>首页侧栏与设置侧栏使用相同的导航条目和分类标签。</div></HubPageShell>} />
      </Routes></main>
    </div>
  </>;
}
const style = document.createElement('style');
style.textContent = `html,body,#root{margin:0;height:100%;min-width:880px;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif}body{background:var(--color-bg-1)}.settings-preview-tools{box-sizing:border-box;min-height:34px;display:flex;align-items:center;gap:8px;padding:0 16px;font-size:11px;color:var(--text-secondary);border-bottom:1px solid var(--color-border-2)}.settings-preview-tools button{font:inherit;border:1px solid var(--color-border-2);border-radius:4px;background:var(--color-bg-1);color:var(--text-primary);cursor:pointer}.settings-preview-frame{height:calc(100% - 34px);display:grid;grid-template-columns:184px minmax(0,1fr)}.settings-preview-frame--minimum{width:880px;height:600px;border:1px solid var(--color-border-2);margin:auto}.settings-preview-frame--collapsed{grid-template-columns:56px minmax(0,1fr)}.settings-preview-frame aside{padding:8px;background:var(--color-fill-1);border-right:1px solid var(--color-border-2);min-height:0}.settings-preview-frame main{display:flex;flex-direction:column;min-height:0;min-width:0}`;
document.head.appendChild(style);
const themeStyle = document.createElement('style');
document.head.appendChild(themeStyle);
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={i18n}><ThemeProvider><ConfigProvider><HashRouter><Preview /></HashRouter></ConfigProvider></ThemeProvider></I18nextProvider>);
