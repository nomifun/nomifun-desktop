import { act, cleanup, fireEvent, render, waitFor, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter, useLocation } from 'react-router-dom';
import { ipcBridge } from '@/common';
import { parseKnowledgeBaseId } from '@/common/types/ids';
import locale from '@/renderer/services/i18n/locales/zh-CN/messages.json';
import conversationLocale from '@/renderer/services/i18n/locales/zh-CN/conversation.json';
import commonLocale from '@/renderer/services/i18n/locales/zh-CN/common.json';
import { Message } from '@arco-design/web-react';
import KnowledgeWritebackNotice from './KnowledgeWritebackNotice';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'zh-CN', resources: { 'zh-CN': { translation: {
  messages: locale, conversation: conversationLocale, common: commonLocale,
} } } });
const kb = parseKnowledgeBaseId('0190f5fe-7c00-7a00-8000-000000000001');
function Location() { return <span data-testid='location'>{useLocation().pathname}</span>; }
afterEach(() => { cleanup(); mock.restore(); });

test('compact notice keeps document details in a popover and opens the exact knowledge base', async () => {
  spyOn(ipcBridge.knowledge.getBase, 'invoke').mockResolvedValue({ name: 'python基础', knowledge_base_id: kb } as never);
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <KnowledgeWritebackNotice notice={{ files: [{ kb_id: kb, rel_path: 'python3-loop-statements.md' }], status: 'written' }} />
    <Location />
  </I18nextProvider></MemoryRouter>);
  const button = page.getByRole('button', { name: '已回写知识库 · 1 篇' });
  expect(button.getAttribute('aria-expanded')).toBe('false');
  expect(page.queryByText('python3-loop-statements.md')).toBeNull();
  await waitFor(() => expect(page.getByText('python基础')).toBeTruthy());
  fireEvent.click(button);
  await waitFor(() => expect(page.getByRole('link', { name: /python3-loop-statements.md/ })).toBeTruthy());
  expect(button.getAttribute('aria-expanded')).toBe('true');
  fireEvent.keyDown(button, { key: 'Escape' });
  expect(button.getAttribute('aria-expanded')).toBe('false');
  expect(document.activeElement).toBe(button);
  fireEvent.click(button);
  fireEvent.click(page.getByRole('link', { name: /python3-loop-statements.md/ }));
  expect(page.getByTestId('location').textContent).toBe(`/knowledge/${kb}`);
});
test('no successful targets produces no notice or metadata request', () => {
  const load = spyOn(ipcBridge.knowledge.getBase, 'invoke');
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <KnowledgeWritebackNotice notice={{ files: [], status: 'written' }} />
  </I18nextProvider></MemoryRouter>);
  expect(page.queryByRole('status')).toBeNull();
  expect(load).not.toHaveBeenCalled();
});

test.each([
  ['running', '正在回写知识库'],
  ['failed', '知识库回写失败'],
  ['cancelled', '知识库回写已取消'],
  ['interrupted', '回写结果未确认'],
  ['no_completer', '知识库回写不可用'],
] as const)('%s is a compact passive status without metadata requests or a false success', (status, label) => {
  const load = spyOn(ipcBridge.knowledge.getBase, 'invoke');
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <KnowledgeWritebackNotice notice={{ status, files: [], event_seq: 12 }} />
  </I18nextProvider></MemoryRouter>);
  expect(page.getByRole('status').textContent).toBe(label);
  expect(page.queryByRole('button')).toBeNull();
  expect(page.queryByText(/已回写知识库/)).toBeNull();
  expect(load).not.toHaveBeenCalled();
});

test('failed write-back keeps typed evidence folded, shows real zero observations and copies sanitized details', async () => {
  const copy = spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
  const load = spyOn(ipcBridge.knowledge.getBase, 'invoke');
  spyOn(Message, 'success').mockImplementation(() => undefined as never);
  spyOn(Message, 'error').mockImplementation(() => undefined as never);
  const secure = Object.getOwnPropertyDescriptor(window, 'isSecureContext');
  Object.defineProperty(window, 'isSecureContext', { configurable: true, value: true });
  try {
    const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
      <KnowledgeWritebackNotice notice={{ status: 'failed', files: [], event_seq: 12, diagnostic: {
        stage: 'first_frame', code: 'USER_LLM_PROVIDER_TIMEOUT', elapsed_ms: 12000,
        received_frames: 0, received_bytes: 0, output_bytes: 0, input_tokens_estimate: 3000, output_token_limit: 1000,
        detail: 'The first frame was not received.',
        provider_diagnostic: { reason: 'request_timeout', modelName: 'actual-model', requestId: 'req_123',
          endpoint: 'https://user:password-secret@api.example.com/v1?token=query-secret#fragment-secret',
          transportDetail: 'Connection failed at https://user:password-secret@api.example.com/v1?token=query-secret\nAuthorization: Bearer auth-secret' },
      } }} />
    </I18nextProvider></MemoryRouter>);
    const button = page.getByRole('button', { name: '知识库回写失败' });
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(page.queryByRole('region')).toBeNull();
    expect(page.queryByText('输入 Token（本地预估）')).toBeNull();
    fireEvent.click(button);
    const region = await page.findByRole('region', { name: '回写详情' });
    const details = within(region);
    expect(details.getByText(conversationLocale.agentError.providerReasons.request_timeout.title)).toBeTruthy();
    for (const value of ['等待模型首帧', 'USER_LLM_PROVIDER_TIMEOUT', '12,000 ms', '3,000', '1,000', 'actual-model', 'https://api.example.com/v1']) {
      expect(region.textContent).toContain(value);
    }
    expect(details.getByText('接收帧数').closest('div')?.querySelector('dd')?.textContent).toBe('0');
    expect(details.getByText('接收字节').closest('div')?.querySelector('dd')?.textContent).toBe('0 B');
    expect(details.getByText('首帧耗时').closest('div')?.querySelector('dd')?.textContent).toBe('未记录');
    expect(details.getByText('输出 Token（服务报告）').closest('div')?.querySelector('dd')?.textContent).toBe('未记录');
    await act(async () => fireEvent.click(details.getByRole('button', { name: conversationLocale.agentError.copyDetails })));
    expect(copy).toHaveBeenCalledTimes(1);
    const report = copy.mock.calls[0]![0];
    for (const value of ['USER_LLM_PROVIDER_TIMEOUT', '等待模型首帧', '3,000', '1,000', 'req_123', 'The first frame was not received.']) expect(report).toContain(value);
    for (const secret of ['password-secret', 'query-secret', 'fragment-secret', 'auth-secret']) {
      expect(region.textContent).not.toContain(secret); expect(report).not.toContain(secret);
    }
    fireEvent.keyDown(button, { key: 'Escape' });
    expect(button.getAttribute('aria-expanded')).toBe('false');
    expect(document.activeElement).toBe(button);
    expect(load).not.toHaveBeenCalled();
  } finally {
    if (secure) Object.defineProperty(window, 'isSecureContext', secure);
    else Reflect.deleteProperty(window, 'isSecureContext');
  }
});

test('old detail is safely folded and arbitrary prose never becomes a provider diagnosis', async () => {
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <KnowledgeWritebackNotice notice={{ status: 'failed', files: [], event_seq: 12,
      detail: 'expired key; 401\nAuthorization: Bearer auth-secret\nhttps://user:password-secret@api.example.com/v1?token=query-secret#fragment-secret',
    }} />
  </I18nextProvider></MemoryRouter>);
  const button = page.getByRole('button', { name: '知识库回写失败' });
  expect(button.getAttribute('title')).toBeNull();
  expect(page.queryByText(/expired key/)).toBeNull();
  fireEvent.click(button);
  const region = await page.findByRole('region', { name: '回写详情' });
  expect(region.textContent).toContain('expired key; 401');
  expect(within(region).queryByText(conversationLocale.agentError.providerReasons.expired_key.title)).toBeNull();
  for (const secret of ['auth-secret', 'password-secret', 'query-secret', 'fragment-secret']) expect(region.textContent).not.toContain(secret);
});

test('unknown global machine codes remain inspectable and missing measurements stay unknown', async () => {
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <KnowledgeWritebackNotice notice={{ status: 'failed', files: [], event_seq: 12,
      diagnostic: { stage: 'write', code: 'KNOWLEDGE_OWNER_REJECTED' },
    }} />
  </I18nextProvider></MemoryRouter>);
  fireEvent.click(page.getByRole('button', { name: '知识库回写失败' }));
  const region = await page.findByRole('region', { name: '回写详情' });
  expect(region.textContent).toContain('KNOWLEDGE_OWNER_REJECTED');
  expect(within(region).getByText('接收帧数').closest('div')?.querySelector('dd')?.textContent).toBe('未记录');
  expect(within(region).getByText('耗时').closest('div')?.querySelector('dd')?.textContent).toBe('未记录');
});

test('startup interruption with only stage and code shows every missing observation as unknown without invented zeroes', async () => {
  const load = spyOn(ipcBridge.knowledge.getBase, 'invoke');
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <KnowledgeWritebackNotice notice={{ status: 'interrupted', files: [], event_seq: 12,
      diagnostic: { stage: 'interrupted', code: 'KNOWLEDGE_WRITEBACK_INTERRUPTED' },
    }} />
  </I18nextProvider></MemoryRouter>);
  const button = page.getByRole('button', { name: '回写结果未确认' });
  expect(button.getAttribute('aria-expanded')).toBe('false');
  expect(page.queryByRole('region')).toBeNull();
  fireEvent.click(button);
  const region = await page.findByRole('region', { name: '回写详情' });
  const details = within(region);
  expect(details.getByText('阶段').closest('div')?.querySelector('dd')?.textContent).toBe('执行中断');
  expect(region.textContent).toContain('KNOWLEDGE_WRITEBACK_INTERRUPTED');
  for (const label of ['耗时', '首帧耗时', '接收帧数', '接收字节', '模型输出字节',
    '输入 Token（本地预估）', '输出 Token（预算上限）', '输出 Token（服务报告）']) {
    expect(details.getByText(label).closest('div')?.querySelector('dd')?.textContent).toBe('未记录');
  }
  expect(details.queryAllByText(/^0(?: ms| B)?$/)).toHaveLength(0);
  expect(details.queryByRole('link')).toBeNull();
  expect(details.getByRole('button', { name: conversationLocale.agentError.copyDetails })).toBeTruthy();
  fireEvent.keyDown(button, { key: 'Escape' });
  expect(button.getAttribute('aria-expanded')).toBe('false');
  expect(document.activeElement).toBe(button);
  expect(load).not.toHaveBeenCalled();
});

test('successful metrics retain file navigation and identify estimates and budgets without claiming an error', async () => {
  spyOn(ipcBridge.knowledge.getBase, 'invoke').mockResolvedValue({ name: 'python基础', knowledge_base_id: kb } as never);
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <KnowledgeWritebackNotice notice={{ status: 'written', files: [{ kb_id: kb, rel_path: 'notes/saved.md' }], event_seq: 12,
      diagnostic: { stage: 'write', elapsed_ms: 1000, first_frame_ms: 100, received_frames: 3,
        received_bytes: 600, output_bytes: 400, input_tokens_estimate: 3000, output_token_limit: 1000, output_tokens: 123 },
    }} />
  </I18nextProvider></MemoryRouter>);
  fireEvent.click(page.getByRole('button', { name: '已回写知识库 · 1 篇' }));
  const region = await page.findByRole('region', { name: '回写详情' });
  expect(within(region).getByRole('link', { name: /notes\/saved.md/ }).getAttribute('href')).toBe(`/knowledge/${kb}`);
  expect(within(region).getByText('输入 Token（本地预估）').closest('div')?.querySelector('dd')?.textContent).toBe('3,000');
  expect(within(region).getByText('输出 Token（预算上限）').closest('div')?.querySelector('dd')?.textContent).toBe('1,000');
  expect(within(region).getByText('输出 Token（服务报告）').closest('div')?.querySelector('dd')?.textContent).toBe('123');
  expect(within(region).queryByText('错误码')).toBeNull();
  expect(region.textContent).not.toContain('失败');
});

test.each(['cancelled', 'interrupted'] as const)('%s keeps already written files available without claiming full success', async status => {
  spyOn(ipcBridge.knowledge.getBase, 'invoke').mockResolvedValue({ name: 'python基础', knowledge_base_id: kb } as never);
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}>
    <KnowledgeWritebackNotice notice={{ status, files: [{ kb_id: kb, rel_path: 'notes/kept.md' }], event_seq: 13 }} />
  </I18nextProvider></MemoryRouter>);
  const button = page.getByRole('button');
  expect(button.textContent).toContain('已写入 1 篇');
  expect(button.textContent).toContain(status === 'cancelled' ? '已取消' : '结果未确认');
  fireEvent.click(button);
  const link = await page.findByRole('link', { name: /notes\/kept.md/ });
  expect(link.getAttribute('href')).toBe(`/knowledge/${kb}`);
});
