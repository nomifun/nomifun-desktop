/** Actual scenario panel/editor with local fixture transport and no account writes. */
import { useLayoutEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { HashRouter } from 'react-router-dom';
import { Button, ConfigProvider } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import zhCN from '@arco-design/web-react/es/locale/zh-CN';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/modal-contract.css';
import settings from '../src/renderer/services/i18n/locales/zh-CN/settings.json';
import common from '../src/renderer/services/i18n/locales/zh-CN/common.json';
import ModelConfigurationPanel, { ModelConfigurationWorkspace } from '../src/renderer/pages/modelHub/ModelConfigurationPanel';
import ModelGatewaySetup from '../src/renderer/pages/settings/components/ModelGatewaySetup';
import ProviderAutoConfigurationNotice from '../src/renderer/pages/settings/components/ProviderAutoConfigurationNotice';
import ProviderCompatibilityModePicker from '../src/renderer/pages/settings/components/ProviderCompatibilityModePicker';
import { emptyCapabilityDraft } from '../src/renderer/pages/settings/components/providerModelAdvanced';
import type { ProviderAutoConfigurationBatch } from '../src/renderer/pages/settings/components/useProviderAutoConfiguration';
import type { ProviderCompatibilityMode } from '../src/renderer/pages/settings/components/providerAutoConfiguration';
import { scenarioProvider, scenarioConnections, scenarioManifests } from './fixtures/scenarioModelEditing';
import type { ProviderResponse } from '../src/common/types/provider/providerApi';
import type { SaveProviderModelRequest } from '../src/common/protocolBindings/SaveProviderModelRequest';
import type { ModelTask } from '../src/common/protocolBindings/ModelTask';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '../src/renderer/pages/settings/DisplaySettings/presets';
import { processCustomCss } from '../src/renderer/utils/theme/customCssProcessor';
import { ensureThemeControlContract } from '../src/renderer/utils/theme/themeControlContract';

let provider = structuredClone(scenarioProvider);
const params = new URLSearchParams(location.search);
const fixtureSettings: Record<string, unknown> = { theme: params.get('mode') === 'dark' ? 'dark' : 'light' };
const probeBatch: ProviderAutoConfigurationBatch = {
  requestBaseUrl: 'https://example.invalid/v1',
  detections: [{ task: 'chat', protocol: 'openai.chat_text', authScheme: 'bearer', confidence: 'reachable' }],
  candidates: [
    { task: 'chat', protocol: 'openai.chat_text', authScheme: 'bearer', confidence: 'reachable' },
    { task: 'chat', protocol: 'anthropic.messages', authScheme: 'header_key:x-api-key', confidence: 'credentials_rejected' },
    { task: 'chat', protocol: 'openai.responses', authScheme: 'bearer', confidence: 'fallback' },
  ],
};
const response = (data: unknown, status = 200) => new Response(JSON.stringify({ success: status < 400, data }), { status, headers: { 'Content-Type': 'application/json' } });
const providerWire = (): ProviderResponse => ({
  provider_id: provider.id, platform: provider.platform, name: provider.name,
  base_url: provider.base_url, auth_scheme: provider.auth_scheme, has_credentials: false,
  models: provider.models, enabled: provider.enabled, sort_order: 0, created_at: 1, updated_at: 1,
});
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(input instanceof Request ? input.url : String(input), location.origin);
  const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
  if (url.pathname === '/api/settings/client') {
    if (method === 'PUT') Object.assign(fixtureSettings, JSON.parse(String(init?.body)));
    return response(fixtureSettings);
  }
  if (url.pathname === '/api/providers') return response([providerWire()]);
  if (url.pathname === '/api/config') return response({});
  if (url.pathname === '/api/providers/model-gateway/meta') return response({ contract_version: '1.0',
    operator: { name: '测试模型网关', homepage_url: null, console_url: null, purchase_url: null, terms_url: null, privacy_url: null },
    capabilities: ['openai-response'], optional_endpoints: [] });
  if (url.pathname === '/api/providers/model-gateway/catalog') return response({ contract_version: '1.0', imports: [], models: [{
    id: 'preview-chat', display_name: '网关模型', vendor: '测试供应商', tasks: ['chat'],
    task_endpoints: { chat: { endpoints: ['openai-response'], preferred_endpoint: 'openai-response' } },
    context_window: 128000, max_output_tokens: 8192, input_modalities: ['text'], traits: [], pricing: [], included_in_plan: true, status: 'available',
  }] });
  if (url.pathname === '/api/model-protocols') return response(scenarioManifests[url.searchParams.get('task') as ModelTask]);
  if (url.pathname === `/api/providers/${provider.id}/connections`) return response(scenarioConnections);
  if (url.pathname.endsWith('/discover-model-protocols')) {
    const { capability } = JSON.parse(String(init?.body));
    return response({ selected_protocol: capability.protocol, selected_capability: capability, elapsed_ms: 0,
      candidates: [{ protocol: capability.protocol, capability, status: 'available', evidence: 'official_model', recommended: true, elapsed_ms: 0 }] });
  }
  if (url.pathname === '/api/provider-models' && method === 'PUT') {
    const request = JSON.parse(String(init?.body)) as SaveProviderModelRequest;
    const previous = provider.models[0];
    provider.models = [{
      ...previous, ...request.model,
      display_name: request.model.display_name,
      description: request.model.description ?? null,
      enabled: request.model.enabled ?? previous.enabled,
      sort_order: request.model.sort_order ?? previous.sort_order,
      capabilities: request.model.capabilities.map((capability) => ({
        ...capability, traits: capability.traits ?? [], provider_params: capability.provider_params ?? {},
        allow_cross_origin_credentials: capability.allow_cross_origin_credentials ?? false, created_at: 11, updated_at: 20,
      })),
    }];
    return response(provider.models[0]);
  }
  return response({}, 404);
}) as typeof fetch;
window.fetch = globalThis.fetch;
class FixtureSocket extends EventTarget {
  static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
  readyState = 1; send() {} close() { this.readyState = 3; }
}
globalThis.WebSocket = FixtureSocket as unknown as typeof WebSocket;

const { default: ModalityModelsPanel } = await import('../src/renderer/pages/modelHub/ModalityModelsPanel');
const { default: ModelHubPage } = await import('../src/renderer/pages/modelHub');
const { MODALITY_SPECS } = await import('../src/renderer/pages/modelHub/modalityModels');
const { ThemeProvider } = await import('../src/renderer/hooks/context/ThemeContext');
const { configService } = await import('../src/common/config/configService');
type Modality = keyof typeof MODALITY_SPECS;
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings, common } } }, interpolation: { escapeValue: false } });
function Preview() {
  const [modality, setModality] = useState<Modality>('chat');
  const [compatibility, setCompatibility] = useState<ProviderCompatibilityMode>('auto');
  const [presetId, setPresetId] = useState(params.get('preset') ?? DEFAULT_THEME_ID);
  const [mode, setMode] = useState<'light' | 'dark'>(fixtureSettings.theme as 'light' | 'dark');
  useLayoutEffect(() => {
    // Match AppThemeRuntime's processed preset and control-contract order. Raw
    // preset CSS misses the !important conflicts this acceptance page exercises.
    const style = document.createElement('style');
    style.textContent = processCustomCss((PRESET_THEMES.find(theme => theme.id === presetId) ?? PRESET_THEMES[0]).css);
    document.head.append(style);
    ensureThemeControlContract();
    return () => style.remove();
  }, [presetId]);
  useLayoutEffect(() => { configService.setLocal('theme', mode); }, [mode]);
  const workspace = params.has('widgets') ? <ModelConfigurationWorkspace>
    <ModelConfigurationPanel visible title='供应商与网关状态样例' onBack={() => {}}>
      <div className='flex flex-col gap-20px'>
        <ProviderCompatibilityModePicker value={compatibility} onChange={setCompatibility} />
        <ProviderAutoConfigurationNotice enabled loading={false} batch={probeBatch}
          definition={{ model: 'preview-model', capabilities: [{ ...emptyCapabilityDraft('chat'), protocol: 'openai.chat_text' }] }}
          onSelect={() => {}} onRecheck={() => {}} />
        <ModelGatewaySetup initialBaseUrl='https://gateway.example.invalid' onCreated={() => {}} onCancel={() => {}} />
      </div>
    </ModelConfigurationPanel>
  </ModelConfigurationWorkspace> : params.has('workspace') ? <ModelHubPage /> : <ModelConfigurationWorkspace><main className='preview-model-page'>
    <div className='preview-note'>共享场景列表与编辑器 · 测试数据 · 不连接供应商、不写入实际配置</div>
    <nav className='flex flex-wrap gap-6px mb-20px'>
      {(Object.keys(MODALITY_SPECS) as Modality[]).map((key) => <Button key={key} size='small' type={key === modality ? 'primary' : 'secondary'} onClick={() => setModality(key)}>
        {key === 'vision' ? '视觉' : i18n.t(`settings.modelTask.${MODALITY_SPECS[key].task}`)}
      </Button>)}
    </nav>
    <ModalityModelsPanel modality={modality} titleKey='settings.modelHub.title' subtitleKey='settings.modelHub.subtitle' />
  </main></ModelConfigurationWorkspace>;
  return <><div className='preview-theme-controls'>
    <span>模型管理视觉验收 · 测试数据</span>
    <label>主题 <select aria-label='验收主题' value={presetId} onChange={event => setPresetId(event.target.value)}>
      {PRESET_THEMES.map(theme => <option key={theme.id} value={theme.id}>{theme.name}</option>)}
    </select></label>
    <label>模式 <select aria-label='验收模式' value={mode} onChange={event => setMode(event.target.value as 'light' | 'dark')}>
      <option value='light'>浅色</option><option value='dark'>深色</option>
    </select></label>
  </div><div className='preview-workspace'>{workspace}</div></>;
}
const style = document.createElement('style');
// Measure settled colors deterministically; animation itself is not under test.
const settledStyles = document.createElement('style');
settledStyles.textContent = '#root .preview-workspace *{transition:none !important;animation:none !important}';
document.head.append(settledStyles);
style.textContent = 'html,body,#root{margin:0;height:100%;min-width:880px;min-height:600px;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif;background:var(--color-bg-1);color:var(--text-primary)}#root{display:flex;flex-direction:column}.preview-workspace{flex:1;min-height:0}.preview-theme-controls{display:flex;align-items:center;gap:20px;padding:10px 16px;border-bottom:1px solid var(--color-border-2);font-size:12px;background:var(--color-bg-1);color:var(--text-secondary)}.preview-theme-controls select{background:var(--color-bg-1);color:var(--text-primary);border:1px solid var(--color-border-3);border-radius:4px;padding:4px}.preview-model-page{padding:24px;max-width:1000px;margin:auto}.preview-note{font-size:12px;color:var(--text-secondary);margin-bottom:20px}';
document.head.append(style);
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={i18n}><ConfigProvider locale={zhCN} theme={{ primaryColor: '#ef2355' }}>
  <SWRConfig value={{ provider: () => new Map(), revalidateOnFocus: false }}><HashRouter><ThemeProvider><Preview /></ThemeProvider></HashRouter></SWRConfig>
</ConfigProvider></I18nextProvider>);
