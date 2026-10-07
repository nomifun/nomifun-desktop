/** Actual scenario panel/editor with local fixture transport and no account writes. */
import { useState } from 'react';
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
import { scenarioProvider, scenarioConnections, scenarioManifests } from './fixtures/scenarioModelEditing';
import type { ProviderResponse } from '../src/common/types/provider/providerApi';
import type { SaveProviderModelRequest } from '../src/common/protocolBindings/SaveProviderModelRequest';
import type { ModelTask } from '../src/common/protocolBindings/ModelTask';

let provider = structuredClone(scenarioProvider);
const response = (data: unknown, status = 200) => new Response(JSON.stringify({ success: status < 400, data }), { status, headers: { 'Content-Type': 'application/json' } });
const providerWire = (): ProviderResponse => ({
  provider_id: provider.id, platform: provider.platform, name: provider.name,
  base_url: provider.base_url, auth_scheme: provider.auth_scheme, has_credentials: false,
  models: provider.models, enabled: provider.enabled, sort_order: 0, created_at: 1, updated_at: 1,
});
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(input instanceof Request ? input.url : String(input), location.origin);
  const method = init?.method ?? (input instanceof Request ? input.method : 'GET');
  if (url.pathname === '/api/providers') return response([providerWire()]);
  if (url.pathname === '/api/config') return response({});
  if (url.pathname === '/api/model-protocols') return response(scenarioManifests[url.searchParams.get('task') as ModelTask]);
  if (url.pathname === `/api/providers/${provider.id}/connections`) return response(scenarioConnections);
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
const { MODALITY_SPECS } = await import('../src/renderer/pages/modelHub/modalityModels');
const { ThemeProvider } = await import('../src/renderer/hooks/context/ThemeContext');
type Modality = keyof typeof MODALITY_SPECS;
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: { settings, common } } }, interpolation: { escapeValue: false } });
function Preview() {
  const [modality, setModality] = useState<Modality>('chat');
  return <main className='preview-model-page'>
    <div className='preview-note'>共享场景列表与编辑器 · 测试数据 · 不连接供应商、不写入实际配置</div>
    <nav className='flex flex-wrap gap-6px mb-20px'>
      {(Object.keys(MODALITY_SPECS) as Modality[]).map((key) => <Button key={key} size='small' type={key === modality ? 'primary' : 'secondary'} onClick={() => setModality(key)}>
        {key === 'vision' ? '视觉' : i18n.t(`settings.modelTask.${MODALITY_SPECS[key].task}`)}
      </Button>)}
    </nav>
    <ModalityModelsPanel modality={modality} titleKey='settings.modelHub.title' subtitleKey='settings.modelHub.subtitle' />
  </main>;
}
const style = document.createElement('style');
style.textContent = 'html,body,#root{margin:0;min-width:880px;min-height:600px;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif;background:var(--color-bg-1)}.preview-model-page{padding:24px;max-width:1000px;margin:auto}.preview-note{font-size:12px;color:var(--color-text-3);margin-bottom:20px}';
document.head.append(style);
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={i18n}><ConfigProvider locale={zhCN} theme={{ primaryColor: '#ef2355' }}>
  <SWRConfig value={{ provider: () => new Map(), revalidateOnFocus: false }}><HashRouter><ThemeProvider><Preview /></ThemeProvider></HashRouter></SWRConfig>
</ConfigProvider></I18nextProvider>);
