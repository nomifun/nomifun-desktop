import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import type { AgentStreamErrorInfo } from '@/common/chat/chatLib';
import type { ModelFailureReason } from '@/common/chat/providerDiagnostic';
import enConversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import zhConversation from '@/renderer/services/i18n/locales/zh-CN/conversation.json';
import enCommon from '@/renderer/services/i18n/locales/en-US/common.json';
import ConversationErrorNote from './ConversationErrorNote';

afterEach(() => { cleanup(); mock.restore(); });

async function mount(error: AgentStreamErrorInfo, language = 'en-US') {
  const i18n = createInstance();
  await i18n.use(initReactI18next).init({ lng: language, resources: {
    'en-US': { translation: { conversation: enConversation, common: enCommon } },
    'zh-CN': { translation: { conversation: zhConversation, common: enCommon } },
  }, interpolation: { escapeValue: false } });
  const result = render(<I18nextProvider i18n={i18n}><ConversationErrorNote
    error={error} sessionId='session-1' turnId='turn-1' timestamp={1234} feedback={false}
  /></I18nextProvider>);
  return { ...result, page: within(result.container) };
}

test.each(['en-US', 'zh-CN'])('distinct key, quota, permission, endpoint and transport evidence replaces generic copy in %s', async language => {
  const locale = language === 'zh-CN' ? zhConversation : enConversation;
  const cases: [ModelFailureReason, number | undefined][] = [
    ['invalid_key', 401], ['expired_key', 401], ['insufficient_quota', 429], ['rate_limited', 429],
    ['auth_failed', 401], ['auth_scheme_mismatch', 401], ['permission_denied', 403], ['model_permission_denied', 403],
    ['endpoint_missing', 404], ['model_not_found', 404], ['non_api_response', 200],
    ['tls_failure', undefined], ['request_timeout', undefined], ['upstream_server_error', 503], ['provider_overloaded', 529],
  ];
  for (const [reason, httpStatus] of cases) {
    const result = await mount({ code: 'USER_LLM_PROVIDER_UNAVAILABLE', message: 'generic failure',
      detail: 'An untrusted diagnostic says: key expired; auth rejected',
      providerDiagnostic: { reason, httpStatus },
    }, language);
    expect(result.page.getByRole('alert').textContent).toBe(locale.agentError.providerReasons[reason].title);
    expect(result.page.getByText(locale.agentError.providerReasons[reason].body).closest('[hidden]')).toBeNull();
    expect(result.page.getByText('An untrusted diagnostic says: key expired; auth rejected').closest('[hidden]')).not.toBeNull();
    expect(result.page.queryByRole('button', { name: 'End this turn' })).toBeNull();
    cleanup();
  }
});

test('request metadata stays folded, uses the actual failing model and copies sanitized technical context', async () => {
  const copy = spyOn(navigator.clipboard, 'writeText').mockResolvedValue(undefined);
  spyOn(Message, 'success').mockImplementation(() => undefined as never);
  spyOn(Message, 'error').mockImplementation(() => undefined as never);
  const secure = Object.getOwnPropertyDescriptor(window, 'isSecureContext');
  Object.defineProperty(window, 'isSecureContext', { configurable: true, value: true });
  try {
  const error: AgentStreamErrorInfo = { message: 'The provider rejected authentication',
    code: 'USER_LLM_PROVIDER_AUTH_FAILED', detail: 'safe original detail', modelName: 'primary-model', agentLabel: 'Test Agent',
    providerDiagnostic: { reason: 'expired_key', providerId: 'provider-2', modelName: 'failover-model', httpStatus: 401,
      endpoint: 'https://user:password-secret@api.example.com/v1?api_key=query-secret#fragment-secret',
      providerCode: 'key_expired', providerType: 'authentication_error', providerParam: 'headers.Authorization',
      requestId: 'req-123', protocol: 'openai_chat', authScheme: 'bearer', retryAfterMs: 0, contentType: 'application/json',
      transportDetail: 'connection failed at https://user:password-secret@api.example.com/v1?token=query-secret',
    } };
  const { page, container } = await mount(error);
  expect(page.queryByRole('button', { name: enConversation.agentError.copyDetails })).toBeNull();
  fireEvent.click(page.getByRole('button', { name: enConversation.agentError.expandDetails }));
  const region = page.getByRole('region');
  for (const value of ['401', 'https://api.example.com/v1', 'key_expired', 'req-123', 'headers.Authorization',
    'failover-model', 'primary-model', 'Session-selected model', 'API protocol', 'Auth method']) expect(region.textContent).toContain(value);
  for (const secret of ['password-secret', 'query-secret', 'fragment-secret']) expect(container.textContent).not.toContain(secret);
  expect(container.querySelector('.message-error-note__detail-body')?.textContent).toBe('connection failed at https://api.example.com/v1');
  await act(async () => { fireEvent.click(page.getByRole('button', { name: enConversation.agentError.copyDetails })); });
  expect(copy).toHaveBeenCalledTimes(1);
  const report = copy.mock.calls[0][0];
  for (const value of ['code: USER_LLM_PROVIDER_AUTH_FAILED', 'reason: expired_key', 'session-1', 'turn-1', 'req-123', 'failover-model']) expect(report).toContain(value);
  for (const secret of ['password-secret', 'query-secret', 'fragment-secret']) expect(report).not.toContain(secret);
  } finally {
    if (secure) Object.defineProperty(window, 'isSecureContext', secure);
    else Reflect.deleteProperty(window, 'isSecureContext');
  }
});

test('an unknown diagnostic does not turn arbitrary text into a key-expired diagnosis', async () => {
  const { page } = await mount({ code: 'USER_LLM_PROVIDER_UNAVAILABLE', message: 'key_expired', detail: 'expired key',
    providerDiagnostic: { reason: 'made_up' } as never,
  });
  expect(page.getByRole('alert').textContent).toBe(enConversation.agentError.codes.USER_LLM_PROVIDER_UNAVAILABLE.title);
});
