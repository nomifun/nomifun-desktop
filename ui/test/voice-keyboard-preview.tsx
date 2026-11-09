/** Isolated UI witness: actual settings page, entirely in-memory fixture effects. */
import { useEffect, useLayoutEffect, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter, Route, Routes, useNavigate } from 'react-router-dom';
import { ConfigProvider } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import zhCN from '@arco-design/web-react/es/locale/zh-CN';
import enUS from '@arco-design/web-react/es/locale/en-US';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/modal-contract.css';
import zh from '../src/renderer/services/i18n/locales/zh-CN';
import en from '../src/renderer/services/i18n/locales/en-US';
import type { KeyboardActionRequest, KeyboardState } from '../src/common/keyboard';
import { DEFAULT_THEME_ID, PRESET_THEMES } from '../src/renderer/pages/settings/DisplaySettings/presets';
import { processCustomCss } from '../src/renderer/utils/theme/customCssProcessor';
import { ensureThemeControlContract } from '../src/renderer/utils/theme/themeControlContract';

type Scenario = 'normal' | 'quota' | 'device-busy' | 'save-failed' | 'missing-input' | 'model-scope';
type LogEntry = { sequence: number; operation: string; detail: string };
const scenarios: Scenario[] = ['normal', 'quota', 'device-busy', 'save-failed', 'missing-input', 'model-scope'];
const params = new URLSearchParams(location.search);
const scenario: Scenario = scenarios.includes(params.get('state') as Scenario) ? params.get('state') as Scenario : 'normal';
const initialLanguage = params.get('lang') === 'en-US' || params.get('lang') === 'en' ? 'en-US' : 'zh-CN';
const config: Record<string, unknown> = {
  theme: params.get('mode') === 'dark' ? 'dark' : 'light', language: initialLanguage,
  languageMode: 'manual', colorScheme: 'default', 'ui.zoomFactor': 1, 'system.keepAwake': false,
};
const callbacks = new Set<(state: KeyboardState) => void>();
const logCallbacks = new Set<() => void>();
const logs: LogEntry[] = [];
const log = (operation: string, detail = '') => {
  logs.push({ sequence: (logs.at(-1)?.sequence ?? 0) + 1, operation, detail });
  if (logs.length > 120) logs.shift();
  for (const callback of logCallbacks) callback();
};
const later = (delay: number, run: () => void) => setTimeout(run, delay);
const copy = <T,>(value: T): T => structuredClone(value);
const response = (data: unknown, status = 200) => new Response(JSON.stringify({ success: status < 400, data }), {
  status, headers: { 'Content-Type': 'application/json' },
});

// Never delegate an unrecognized call to native APIs or a running backend.
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(input instanceof Request ? input.url : String(input), location.origin);
  const method = (init?.method ?? (input instanceof Request ? input.method : 'GET')).toUpperCase();
  if (url.origin === location.origin && url.pathname === '/api/settings/client' && ['GET', 'PUT'].includes(method)) {
    if (method === 'PUT') {
      const patch = JSON.parse(typeof init?.body === 'string' ? init.body : '{}') as Record<string, unknown>;
      Object.assign(config, patch);
      log('mock client-settings write', Object.keys(patch).join(', '));
    }
    return response(config);
  }
  log('blocked fetch · 404', `${method} ${url.pathname}`);
  return response('Unsupported isolated preview request', 404);
}) as typeof fetch;
window.fetch = globalThis.fetch;
const BrowserSocket = globalThis.WebSocket;
globalThis.WebSocket = class PreviewSocket extends BrowserSocket {
  constructor(url: string | URL, protocols?: string | string[]) {
    const target = new URL(String(url), location.href);
    // Only Vite's own loopback HMR channel can leave this document.
    if (target.host !== location.host || target.pathname !== '/') throw new Error('Preview blocks backend WebSocket connections');
    super(url, protocols);
  }
};

const { keyboard, defaultVoiceKeyboardShortcuts } = await import('../src/common/keyboard');
const { ipcBridge } = await import('../src/common');
const { configService } = await import('../src/common/config/configService');
const { ThemeProvider, useThemeContext } = await import('../src/renderer/hooks/context/ThemeContext');
const { default: KeyboardSettings } = await import('../src/renderer/pages/settings/KeyboardSettings');
const { default: SettingsSider } = await import('../src/renderer/pages/settings/components/SettingsSider');
const idleMicrophone = () => ({ id: null, deviceId: null, state: 'idle' as const, callbackObserved: false,
  samples: 0, nonzero: 0, peak: 0, rms: 0, elapsedMs: 0, stopped: true, detail: null });
const quotaError = '无法开始语音识别，尚未开始录音。语音识别服务额度不足，请检查账户余额及配额。详情：HTTP 402 · Simulated quota exceeded; check plan and billing details.';
const occupiedError = '模拟端口 PREVIEW-COM 无法打开：拒绝访问。请关闭其他占用键盘的 NomiFun 实例后重试。';
function initialState(): KeyboardState {
  return {
    sequence: 1, cancelGeneration: 0, documentId: 'preview-document', revision: 0,
    operationId: null, editSessionId: null, dictationSelection: null,
    settings: { enabled: true, externalKeyboardEnabled: scenario === 'device-busy', shortcuts: { ...defaultVoiceKeyboardShortcuts },
      k2Enabled: true, k3Enabled: true, k4Enabled: true, audioDeviceId: null, asrMode: scenario === 'model-scope' ? 'platform' : 'streaming',
      asrModel: scenario === 'model-scope' ? null : { providerId: 'preview-provider', model: 'preview-live-asr' },
      textModel: { providerId: 'preview-provider', model: 'preview-polish' }, language: 'zh', dictionary: [],
      beautifyInstruction: '整理标点和段落，保留原意。', alwaysOnTop: false, subtitleFontSize: 14 },
    phase: 'idle', text: '', originalText: '', partial: '', candidate: '', visible: false,
    pendingAction: null, error: scenario === 'quota' ? quotaError : null, notice: null,
    audioState: { state: 'stopped', detail: null }, capturePreparing: false, microphoneTest: idleMicrophone(),
    device: { connected: false, id: null, port: null, firmware: null, policyState: 'applied', captureState: 'stopped', detail: null },
    service: { running: true, ready: true, detail: null }, shortcutState: { registered: 4, detail: null },
    externalKeyboard: scenario === 'device-busy' ? { state: 'error', detail: occupiedError } : { state: 'disabled', detail: null },
  };
}
let state = initialState();
const publish = (patch: Partial<KeyboardState> = {}) => {
  state = { ...state, ...patch, sequence: state.sequence + 1 };
  for (const callback of callbacks) callback(copy(state));
  return copy(state);
};
let cloudOwner: string | null = null;
let cloudBaseline = '';
function finishCloud(cancelled = false) {
  if (!cloudOwner) return;
  cloudOwner = null;
  log(cancelled ? 'mock cloud test cancelled' : 'mock cloud test completed', 'synthetic text only; no network/audio');
  const text = cancelled ? cloudBaseline : `${cloudBaseline}语音检查完成：这是完全模拟的识别结果。`;
  publish({ phase: 'idle', capturePreparing: false, audioState: { state: 'stopped', detail: null }, operationId: null,
    text, originalText: text, revision: cancelled ? state.revision : state.revision + 1,
    partial: '', error: null, ...(cancelled ? { cancelGeneration: state.cancelGeneration + 1 } : {}) });
}
keyboard.available = () => true;
keyboard.getState = async () => copy(state);
keyboard.listen = async callback => { callbacks.add(callback); return () => { callbacks.delete(callback); }; };
keyboard.listenEditFlush = async () => () => {};
keyboard.getCatalog = async () => ({
  asr: scenario === 'model-scope'
    ? [{ providerId: 'preview-stepfun', model: 'stepaudio-2.5-asr-stream', label: 'StepFun · stepaudio-2.5-asr-stream（模拟目录）', protocol: 'stepfun.asr_stream' }]
    : [{ providerId: 'preview-provider', model: 'preview-live-asr', label: '模拟实时识别 · Preview live ASR', protocol: 'stepfun.asr_stream' }],
  platformAsr: [{ providerId: 'preview-provider', model: 'preview-file-asr', label: '模拟普通识别 · Preview file ASR', protocol: 'openai.audio_transcriptions' }],
  text: [{ providerId: 'preview-provider', model: 'preview-polish', label: '模拟文字润色 · Preview text model', protocol: 'openai.chat_text' }],
});
keyboard.getAudioDevices = async () => scenario === 'missing-input' ? [] : [
  { id: 'preview-microphone', label: '模拟麦克风 · Preview microphone', isDefault: true },
  { id: 'preview-secondary', label: '模拟备用输入 · Preview secondary input', isDefault: false },
];
keyboard.updateSettings = async next => {
  const changed = Object.keys(next).filter(key => JSON.stringify(next[key as keyof typeof next]) !== JSON.stringify(state.settings[key as keyof typeof next]));
  log('mock settings write requested', changed.join(', ') || 'unchanged');
  if (scenario === 'save-failed') { log('mock settings write rejected', 'saved settings remain unchanged'); throw new Error('模拟保存失败：无法写入配置文件；上次保存的设置仍然有效。'); }
  const externalChanged = next.externalKeyboardEnabled !== state.settings.externalKeyboardEnabled;
  publish({ settings: copy(next), error: null, shortcutState: { registered: next.enabled ? Object.values(next.shortcuts).filter(Boolean).length : 0, detail: null } });
  if (externalChanged) {
    const enabled = next.externalKeyboardEnabled;
    publish({ externalKeyboard: { state: enabled ? 'starting' : 'stopping', detail: null } });
    later(450, () => {
      if (state.settings.externalKeyboardEnabled !== enabled) return;
      if (enabled && scenario === 'device-busy') publish({ externalKeyboard: { state: 'error', detail: occupiedError } });
      else publish({ externalKeyboard: { state: enabled ? 'connected' : 'disabled', detail: null },
        device: { ...state.device, connected: enabled, id: enabled ? 'PREVIEW-DEVICE' : null,
          port: enabled ? 'PREVIEW-COM' : null, firmware: enabled ? 'fixture-0.3' : null, detail: null } });
      log('mock external adapter settled', enabled ? 'simulated connection only' : 'disabled');
    });
  }
  log('mock settings write accepted', changed.join(', '));
  return copy(state);
};
keyboard.action = async (request: KeyboardActionRequest) => {
  log('mock action', request.action);
  if (request.action === 'microphone_test_start') {
    if (scenario === 'missing-input') throw new Error('模拟环境中没有可用输入设备。');
    const id = request.testId ?? 'preview-local-check';
    publish({ microphoneTest: { ...idleMicrophone(), id, deviceId: request.audioDeviceId ?? null, state: 'starting', stopped: false }, audioState: { state: 'starting', detail: null } });
    later(150, () => { if (state.microphoneTest.id === id && state.microphoneTest.state === 'starting') publish({ microphoneTest: { ...state.microphoneTest, state: 'listening', callbackObserved: true, samples: 3200, nonzero: 2600, peak: 0.63, rms: 0.28, elapsedMs: 200 }, audioState: { state: 'capturing', detail: null } }); });
    later(1200, () => { if (state.microphoneTest.id === id && state.microphoneTest.state === 'listening') publish({ microphoneTest: { ...state.microphoneTest, samples: 19200, nonzero: 18000, elapsedMs: 1200, peak: 0.42, rms: 0.17 } }); });
    later(5000, () => { if (state.microphoneTest.id === id && state.microphoneTest.state === 'listening') { publish({ microphoneTest: { ...state.microphoneTest, state: 'completed', stopped: true, samples: 80000, nonzero: 72000, elapsedMs: 5000, peak: 0.58, rms: 0.24 }, audioState: { state: 'stopped', detail: null } }); log('mock local test completed', 'synthetic callback/sample evidence; no OS microphone'); } });
  } else if (request.action === 'microphone_test_stop') {
    if (state.microphoneTest.id === request.testId && !state.microphoneTest.stopped) {
      const id = request.testId;
      publish({ microphoneTest: { ...state.microphoneTest, state: 'stopping' }, audioState: { state: 'stopping', detail: null } });
      later(180, () => { if (state.microphoneTest.id === id) publish({ microphoneTest: { ...state.microphoneTest, state: 'cancelled', stopped: true }, audioState: { state: 'stopped', detail: null } }); });
    }
  } else if (request.action === 'test_start') {
    cloudOwner = crypto.randomUUID(); cloudBaseline = state.text;
    const owner = cloudOwner;
    publish({ capturePreparing: true, audioState: { state: 'starting', detail: null }, error: null });
    later(250, () => {
      if (cloudOwner !== owner) return;
      if (scenario === 'quota') {
        cloudOwner = null;
        publish({ capturePreparing: false, audioState: { state: 'stopped', detail: null }, error: quotaError });
        log('mock cloud test rejected', 'HTTP 402 fixture; no microphone owner');
      } else publish({ visible: true, phase: 'recording', operationId: owner, capturePreparing: false, audioState: { state: 'capturing', detail: null } });
    });
    later(2400, () => { if (cloudOwner === owner) finishCloud(); });
  } else if (request.action === 'test_stop' || request.action === 'stop') finishCloud();
  else if (request.action === 'cancel') finishCloud(true);
  else if (request.action === 'open') publish({ visible: true });
  else if (request.action === 'close') publish({ visible: false });
  return copy(state);
};
const permissionStatus = () => ({ platform: 'windows' as const, app_label: 'NomiFun isolated UI preview', permissions: [{
  kind: 'microphone' as const, state: 'granted' as const, can_request: false, can_open_settings: true,
  requires_restart_after_grant: false, capabilities: ['voice_input' as const],
}] });
ipcBridge.systemPermissions.get.invoke = async () => { log('mock permissions read'); return permissionStatus(); };
ipcBridge.systemPermissions.request.invoke = async ({ kind }) => { log('mock permission request', kind); return permissionStatus(); };
ipcBridge.systemPermissions.openSettings.invoke = async ({ kind }) => { log('mock open system settings', kind); };
ipcBridge.application.applyKeepAwake.invoke = async () => { log('mock keep-awake ignored'); };
ipcBridge.application.setZoomFactor.invoke = async ({ factor }) => factor;
ipcBridge.mode.onProvidersChanged.on = () => () => {};
ipcBridge.conversation.reconnected.on = () => () => {};
await configService.initialize();
const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: initialLanguage, fallbackLng: 'zh-CN',
  resources: { 'zh-CN': { translation: zh }, 'en-US': { translation: en } }, interpolation: { escapeValue: false } });

function Preview() {
  const navigate = useNavigate();
  const { theme, setTheme } = useThemeContext();
  const [language, setLanguage] = useState<string>(initialLanguage);
  const [logRevision, setLogRevision] = useState(0);
  const [showLog, setShowLog] = useState(params.get('logs') === '1');
  useEffect(() => { const update = () => setLogRevision(value => value + 1); logCallbacks.add(update); update(); return () => { logCallbacks.delete(update); }; }, []);
  useLayoutEffect(() => {
    const style = document.createElement('style');
    style.dataset.voicePreviewPreset = DEFAULT_THEME_ID;
    style.textContent = processCustomCss(PRESET_THEMES.find(preset => preset.id === DEFAULT_THEME_ID)!.css);
    document.head.append(style);
    ensureThemeControlContract();
    return () => style.remove();
  }, []);
  const chooseScenario = (next: Scenario) => {
    // Autosave recovery intentionally outlives a React remount in the product.
    // Each fixture scenario therefore owns a fresh document/module lifetime.
    const target = new URL(location.href);
    target.searchParams.set('state', next);
    target.searchParams.set('mode', theme);
    target.searchParams.set('lang', language);
    if (showLog) target.searchParams.set('logs', '1');
    else target.searchParams.delete('logs');
    location.assign(target.href);
  };
  return <ConfigProvider locale={language === 'en-US' ? enUS : zhCN}>
    <div className='voice-preview-toolbar' data-voice-preview-toolbar>
      <strong>隔离预览 · Mock only</strong>
      <label>主题 <select aria-label='Preview theme' value={theme} onChange={event => { void setTheme(event.target.value as 'light' | 'dark'); }}><option value='light'>浅色 / Light</option><option value='dark'>深色 / Dark</option></select></label>
      <label>语言 <select aria-label='Preview language' value={language} onChange={event => { const next = event.target.value; setLanguage(next); void i18n.changeLanguage(next); document.documentElement.lang = next; }}><option value='zh-CN'>中文</option><option value='en-US'>English</option></select></label>
      <label>场景 <select aria-label='Preview scenario' value={scenario} onChange={event => chooseScenario(event.target.value as Scenario)}>
        <option value='normal'>正常 / Ready</option><option value='quota'>额度不足 / Quota</option><option value='device-busy'>设备占用 / Busy device</option><option value='save-failed'>保存失败 / Save fails on change</option><option value='missing-input'>无输入 / Missing input</option><option value='model-scope'>模型范围 / Model scope</option>
      </select></label>
      <button onClick={() => setShowLog(value => !value)} aria-expanded={showLog} aria-controls='voice-preview-log'>操作日志 ({logs.length})</button>
    </div>
    <div className='voice-preview-scope' role='note'>真实设置页面 · 默认经典预设 · 所有配置、权限、设备、快捷键、音频与模型行为均为内存模拟。
      {scenario === 'save-failed' && <span> 当前修改任一设置即模拟写入失败，原已保存值不变。</span>}
    </div>
    <div className='voice-preview-shell'>
      <aside className='voice-preview-sidebar'><SettingsSider /></aside>
      <main className='voice-preview-main'><SWRConfig value={{ provider: () => new Map(), revalidateOnFocus: false, shouldRetryOnError: false }}>
        <Routes><Route path='/settings/voice-keyboard' element={<KeyboardSettings />} />
          <Route path='*' element={<div className='voice-preview-placeholder'><p>此独立夹具只预览语音键盘；该导航未接入真实页面或后端。</p><button onClick={() => navigate('/settings/voice-keyboard')}>返回语音键盘 / Back</button></div>} />
        </Routes>
      </SWRConfig></main>
    </div>
    {showLog && <aside id='voice-preview-log' className='voice-preview-log' data-preview-log data-revision={logRevision} aria-label='Mock operation log'>
      <div><strong>仅模拟调用 · No real writes</strong><button onClick={() => { logs.length = 0; setLogRevision(value => value + 1); }}>清空 / Clear</button></div>
      <ol>{logs.map(entry => <li key={entry.sequence}><code>{entry.operation}</code>{entry.detail && <span> · {entry.detail}</span>}</li>)}</ol>
    </aside>}
  </ConfigProvider>;
}
const style = document.createElement('style');
style.textContent = `html,body,#root{margin:0;height:100%;min-width:880px;min-height:600px;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif;background:var(--color-bg-1);color:var(--text-primary)}#root{display:flex;flex-direction:column}.voice-preview-toolbar{display:flex;align-items:center;flex-wrap:wrap;gap:8px 16px;box-sizing:border-box;min-height:40px;padding:6px 12px;border-bottom:1px solid var(--color-border-2);font-size:11px;flex-shrink:0}.voice-preview-toolbar label{display:flex;align-items:center;gap:6px}.voice-preview-toolbar select,.voice-preview-toolbar button,.voice-preview-log button,.voice-preview-placeholder button{font:inherit;color:var(--text-primary);background:var(--color-bg-1);border:1px solid var(--color-border-3);border-radius:5px;padding:3px 7px}.voice-preview-scope{padding:4px 12px;border-bottom:1px solid var(--color-border-2);color:var(--text-secondary);font-size:11px;line-height:18px;flex-shrink:0}.voice-preview-shell{display:grid;grid-template-columns:224px minmax(0,1fr);flex:1;min-height:0}.voice-preview-sidebar{padding:12px 10px;border-right:1px solid var(--color-border-2);background:var(--color-fill-1);min-width:0;min-height:0}.voice-preview-main{display:flex;flex-direction:column;min-width:0;min-height:0}.voice-preview-placeholder{padding:24px;font-size:13px}.voice-preview-log{flex-shrink:0;max-height:190px;overflow:auto;border-top:1px solid var(--color-border-2);background:var(--color-bg-1);padding:8px 12px;font-size:11px;line-height:1.6}.voice-preview-log>div{display:flex;align-items:center;justify-content:space-between}.voice-preview-log ol{margin:6px 0 0;padding-left:24px}.voice-preview-log li{overflow-wrap:anywhere}.voice-preview-log code{font-family:ui-monospace,Consolas,monospace}.voice-preview-toolbar :focus-visible{outline:2px solid var(--control-focus-ring);outline-offset:2px}`;
document.head.append(style);
log('fixture initialized', scenario);
createRoot(document.getElementById('root')!).render(<I18nextProvider i18n={i18n}><ThemeProvider>
  <MemoryRouter initialEntries={['/settings/voice-keyboard']}><Preview /></MemoryRouter>
</ThemeProvider></I18nextProvider>);
