/// <reference types="vite/client" />
/** Development-only desktop fixture using production components and fixed data. */
import { useEffect, useRef, useState } from 'react';
import { createRoot } from 'react-dom/client';
import { MemoryRouter } from 'react-router-dom';
import { ConfigProvider, Switch } from '@arco-design/web-react';
import '@arco-design/web-react/es/_util/react-19-adapter';
import arcoZh from '@arco-design/web-react/es/locale/zh-CN';
import arcoEn from '@arco-design/web-react/es/locale/en-US';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import 'virtual:uno.css';
import '@arco-design/web-react/dist/css/arco.css';
import '../src/renderer/styles/arco-override.css';
import '../src/renderer/styles/themes/index.css';
import '../src/renderer/styles/feedback-bubble-contract.css';
import type { IProvider } from '../src/common/config/storage';
import type { IMessageText, IMessageTips, TMessage } from '../src/common/chat/chatLib';
import type { IIdmmConfig } from '../src/common/types/idmm';
import zhLocales from '../src/renderer/services/i18n/locales/zh-CN/index';
import enLocales from '../src/renderer/services/i18n/locales/en-US/index';

// Everything below is isolated to this preview document. Requests never fall
// through to a real backend, and settings writes only update this in-memory map.
const preferences: Record<string, unknown> = { 'chat.idmm.showDecisionBasis': false };
globalThis.fetch = (async (input: RequestInfo | URL, init?: RequestInit) => {
  const url = new URL(typeof input === 'string' ? input : input instanceof URL ? input.href : input.url, location.href);
  if (url.pathname === '/api/settings/client') {
    if (init?.method === 'PUT' && typeof init.body === 'string') Object.assign(preferences, JSON.parse(init.body));
    return new Response(JSON.stringify({ success: true, data: preferences }), { headers: { 'Content-Type': 'application/json' } });
  }
  return new Response(JSON.stringify({ success: false, error: 'Preview transport is isolated.' }), {
    status: 403, headers: { 'Content-Type': 'application/json' },
  });
}) as typeof fetch;

// Preserve Vite HMR, but never connect the application's /ws transport.
const NativeWebSocket = globalThis.WebSocket;
class PreviewWebSocket extends EventTarget {
  static CONNECTING = 0; static OPEN = 1; static CLOSING = 2; static CLOSED = 3;
  readyState = PreviewWebSocket.CLOSED;
  onopen = null; onclose = null; onerror = null; onmessage = null;
  constructor(url: string | URL, protocols?: string | string[]) {
    super();
    if (new URL(String(url), location.href).pathname !== '/ws') {
      return new NativeWebSocket(url, protocols) as unknown as PreviewWebSocket;
    }
  }
  send() {} close() {}
}
globalThis.WebSocket = PreviewWebSocket as unknown as typeof WebSocket;

const { ipcBridge } = await import('../src/common');
const { parseConversationId, parseMessageId, parseProviderId } = await import('../src/common/types/ids');
const { createDefaultIdmmConfig, normalizeIdmmDecisionExplanation } = await import('../src/common/types/idmm');
const { configService } = await import('../src/common/config/configService');
const { useConfig } = await import('../src/renderer/hooks/config/useConfig');
const { ConversationProvider } = await import('../src/renderer/hooks/context/ConversationContext');
const { MessageListProvider } = await import('../src/renderer/pages/conversation/Messages/hooks');
const { default: MessageText } = await import('../src/renderer/pages/conversation/Messages/components/MessageText');
const { default: MessageTips } = await import('../src/renderer/pages/conversation/Messages/components/MessageTips');
const { default: IdmmControl } = await import('../src/renderer/pages/conversation/components/IdmmControl');
const { emitter } = await import('../src/renderer/utils/emitter');
const { CHAT_MESSAGE_JUMP_EVENT } = await import('../src/renderer/utils/chat/chatMinimapEvents');

const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000501');
const providerId = parseProviderId('0190f5fe-7c00-7a00-8000-000000000502');
const id = (index: number) => parseMessageId(`0190f5fe-7c00-7a00-8000-${String(index).padStart(12, '0')}`);
const time = Date.UTC(2026, 9, 7, 4, 0, 0);
const modelName = 'step-3.7-flash';
const providers: IProvider[] = [{
  id: providerId, platform: 'openai', name: 'Fixture provider', base_url: 'https://example.invalid',
  auth_scheme: 'bearer', has_credentials: false, enabled: true,
  models: [{ provider_id: providerId, model: modelName, enabled: true, sort_order: 0, created_at: time, updated_at: time,
    capabilities: [{ task: 'chat', traits: [], protocol: 'openai.chat_text', connection_role: 'default',
      allow_cross_origin_credentials: false, provider_params: {}, created_at: time, updated_at: time }],
  }],
}];
ipcBridge.mode.listProviders.invoke = async () => providers;
ipcBridge.mode.onProvidersChanged.on = () => () => {};
ipcBridge.conversation.reconnected.on = () => () => {};
await configService.initialize();

const question = (index: number) => ({ message_id: id(index), sequence: index, fingerprint: 'a'.repeat(64) });
const explanation = (value: unknown) => {
  const validated = normalizeIdmmDecisionExplanation(value);
  if (!validated) throw new Error('Preview explanation does not satisfy the canonical wire contract.');
  return validated;
};
const human: IMessageText = { id: 'human', message_id: id(510), msg_id: id(510), conversation_id: conversationId,
  type: 'text', position: 'right', status: 'finish', created_at: time,
  content: { content: '先检查现有缓存结构，采用最小改动继续实现。' } };
const rule: IMessageText = { ...human, id: 'rule', message_id: id(511), msg_id: id(511), created_at: time + 1000,
  content: { content: '2', idmm_decision: explanation({ intervention_id: id(521), source: 'rule',
    reason_code: 'rule_selected_recommended_option', rationale: '候选项包含明确推荐的安全选项，按规则采用该项继续。', question: question(531) }) } };
const bypass: IMessageText = { ...human, id: 'bypass', message_id: id(512), msg_id: id(512), created_at: time + 2000,
  content: { content: '采用 LRU 和 30 分钟 TTL，沿用现有缓存结构。', idmm_decision: explanation({
    intervention_id: id(522), source: 'bypass_model', reason_code: 'bypass_model_decision',
    rationale: '沿用现有结构，并先使用有界缓存默认值。', model: { provider_id: providerId, model: modelName }, question: question(532),
  }) } };
const waiting: IMessageTips = { ...human, id: 'waiting', message_id: id(513), msg_id: id(513), position: 'center', type: 'tips', created_at: time + 3000,
  content: { content: '', type: 'warning', idmm_notice: { status: 'waiting_for_human', created_at: time + 3000,
    decision: explanation({ intervention_id: id(523), source: 'rule', reason_code: 'sensitive_input_required',
      rationale: '问题涉及权限，需要由你处理。', question: question(533) }),
  } } };
const failed: IMessageTips = { ...human, id: 'failed', message_id: id(514), msg_id: id(514), position: 'center', type: 'tips', created_at: time + 4000,
  content: { content: '', type: 'error', idmm_notice: { status: 'failed', created_at: time + 4000,
    decision: explanation({ intervention_id: id(524), source: 'bypass_model', reason_code: 'bypass_model_failed',
      rationale: '旁路模型未返回有效决策，本次没有自动发送答案。', model: { provider_id: providerId, model: modelName }, question: question(534) }),
  } } };
const messages: TMessage[] = [human, rule, bypass, waiting, failed];
const questions: IMessageText[] = [
  ['请选择下一步：\n1. 删除缓存\n2. 继续分析（推荐）', 531],
  ['你希望缓存策略怎么设计？', 532],
  ['此步骤需要修改系统权限，请由你确认后继续。', 533],
  ['缓存失效条件尚未确定，你希望如何处理？', 534],
].map(([content, index]) => ({ ...human, id: `question-${index}`, message_id: id(Number(index)), msg_id: id(Number(index)),
  position: 'left', content: { content: String(content) } }));

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: {
  'zh-CN': { translation: zhLocales }, 'en-US': { translation: enLocales },
}, interpolation: { escapeValue: false } });

function Preview() {
  const container = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLTextAreaElement>(null);
  const rawQuestions = useRef<HTMLDetailsElement>(null);
  const [theme, setTheme] = useState<'light' | 'dark'>('light');
  const [language, setLanguage] = useState<'zh-CN' | 'en-US'>('zh-CN');
  const [showBasis, setShowBasis] = useConfig('chat.idmm.showDecisionBasis');
  const [policy, setPolicy] = useState<IIdmmConfig>(() => ({ ...createDefaultIdmmConfig(), mode: 'rule_plus_model',
    bypass_model: { provider_id: providerId, model: modelName } }));
  useEffect(() => {
    document.documentElement.setAttribute('data-theme', theme);
    document.documentElement.setAttribute('data-color-scheme', 'default');
    document.body.setAttribute('arco-theme', theme);
  }, [theme]);
  useEffect(() => {
    const focus = (owner: typeof conversationId) => { if (owner === conversationId) input.current?.focus(); };
    emitter.on('sendbox.focus', focus);
    const jump = (event: Event) => {
      const detail = (event as CustomEvent<{ conversation_id: string; messageId?: string }>).detail;
      if (detail.conversation_id !== conversationId || !detail.messageId) return;
      if (rawQuestions.current) rawQuestions.current.open = true;
      container.current?.querySelector<HTMLElement>(`[data-message-business-id="${detail.messageId}"]`)
        ?.scrollIntoView({ behavior: 'smooth', block: 'center' });
    };
    window.addEventListener(CHAT_MESSAGE_JUMP_EVENT, jump);
    return () => { emitter.off('sendbox.focus', focus); window.removeEventListener(CHAT_MESSAGE_JUMP_EVENT, jump); };
  }, []);
  return <ConfigProvider locale={language === 'zh-CN' ? arcoZh : arcoEn} getPopupContainer={() => container.current || document.body}>
    <div className='preview-frame' ref={container}>
      <header className='preview-header'>
        <div><h1>智能决策 · 真实消息组件预览</h1><p>仅使用固定测试数据 · 不连接后台、不调用模型、不发送消息</p></div>
        <div className='preview-toolbar'>
          <label>依据默认展开 <Switch size='small' checked={showBasis === true} onChange={value => void setShowBasis(value)} /></label>
          <button type='button' onClick={() => setTheme(theme === 'light' ? 'dark' : 'light')}>{theme === 'light' ? '切换深色' : '切换浅色'}</button>
          <button type='button' onClick={() => {
            const next = language === 'zh-CN' ? 'en-US' : 'zh-CN'; setLanguage(next); void i18n.changeLanguage(next);
          }}>{language === 'zh-CN' ? 'English' : '中文'}</button>
        </div>
      </header>
      <div className='preview-layout'>
        <ConversationProvider value={{ conversation_id: conversationId, type: 'nomi', isProcessing: false }}>
          <MessageListProvider initialValue={messages}>
            <main className='preview-timeline' aria-label='消息预览'>
              <div className='preview-row'><span className='preview-label'>人工原文</span><MessageText message={human} /></div>
              <div className='preview-row'><span className='preview-label'>确定性规则选择</span><MessageText message={rule} /></div>
              <div className='preview-row'><span className='preview-label'>旁路模型回答</span><MessageText message={bypass} /></div>
              <MessageTips message={waiting} />
              <MessageTips message={failed} />
              <details ref={rawQuestions} className='preview-questions'><summary>原问题（测试引用）</summary>
                {questions.map(value => <section key={value.id} data-message-business-id={value.message_id}><MessageText message={value} /></section>)}
              </details>
              <textarea ref={input} className='preview-input' aria-label='测试输入框，不会发送' placeholder='“补充输入”仅聚焦此测试输入框，不会发送' />
            </main>
          </MessageListProvider>
        </ConversationProvider>
        <aside className='preview-policy'><h2>运行策略</h2><IdmmControl presentation='embedded' draft={{ value: policy, onChange: setPolicy }} /></aside>
      </div>
    </div>
  </ConfigProvider>;
}

const style = document.createElement('style');
style.textContent = `html,body,#root{margin:0;height:100%;min-width:880px;min-height:600px;font-family:Inter,"Microsoft YaHei",system-ui,sans-serif;background:var(--bg-base);color:var(--text-primary)}
*{box-sizing:border-box}.preview-frame{height:100%;max-width:1320px;margin:auto;padding:18px 22px;display:flex;flex-direction:column;gap:16px}.preview-header{display:flex;align-items:center;justify-content:space-between;gap:20px;flex-shrink:0}.preview-header h1{margin:0;font-size:17px}.preview-header p{margin:5px 0 0;font-size:11px;color:var(--text-secondary)}.preview-toolbar{display:flex;align-items:center;gap:12px;font-size:12px}.preview-toolbar label{display:flex;align-items:center;gap:8px}.preview-toolbar button{font:inherit;color:var(--text-primary);background:var(--bg-2);border:1px solid var(--border-base);border-radius:6px;padding:6px 9px;cursor:pointer}.preview-layout{display:grid;grid-template-columns:minmax(0,1fr) 304px;min-height:0;flex:1;gap:22px}.preview-timeline{min-width:0;overflow:auto;display:flex;flex-direction:column;gap:12px;padding:14px 16px;border:1px solid var(--border-base);border-radius:12px}.preview-row{display:flex;flex-direction:column;gap:4px}.preview-label{font-size:10px;color:var(--text-secondary)}.preview-policy{min-width:0;overflow:auto;padding:12px;border:1px solid var(--border-base);border-radius:12px;background:var(--bg-1)}.preview-policy h2{font-size:12px;margin:0 0 12px}.preview-questions{font-size:12px;color:var(--text-secondary)}.preview-questions summary{cursor:pointer}.preview-questions section{padding:12px 0}.preview-input{min-height:50px;width:100%;resize:vertical;border:1px solid var(--border-base);border-radius:8px;background:var(--bg-base);color:var(--text-primary);padding:10px;font:inherit;font-size:12px;flex-shrink:0}`;
document.head.appendChild(style);
const root = createRoot(document.getElementById('root')!);
if (import.meta.hot) import.meta.hot.dispose(() => root.unmount());
root.render(<I18nextProvider i18n={i18n}><MemoryRouter><Preview /></MemoryRouter></I18nextProvider>);
