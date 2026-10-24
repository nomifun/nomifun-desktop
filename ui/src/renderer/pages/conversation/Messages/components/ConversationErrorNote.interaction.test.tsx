import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import type { AgentStreamErrorInfo, IMessageAgentStatus } from '@/common/chat/chatLib';
import type { ModelFailureReason } from '@/common/chat/providerDiagnostic';
import enConversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import zhConversation from '@/renderer/services/i18n/locales/zh-CN/conversation.json';
import enCommon from '@/renderer/services/i18n/locales/en-US/common.json';
import ConversationErrorNote from './ConversationErrorNote';
import MessageAgentStatus from './MessageAgentStatus';
import { parseConversationId } from '@/common/types/ids';
import { BackendHttpError } from '@/common/adapter/httpBridge';
import { conversationRequestError } from '../../utils/conversationRequestError';

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

test.each(['en-US', 'zh-CN'])('a rejected completion report names the local check and exposes its cause in %s', async language => {
  const locale = language === 'zh-CN' ? zhConversation : enConversation;
  const { page, container } = await mount({ message: 'Completion report rejected',
    code: 'NOMIFUN_TASK_INCOMPLETE', ownership: 'nomifun', retryable: false,
    failureStage: 'completion_report', taskIncompleteReason: 'completion_report_rejected',
    detail: 'Required SubAgent report was missing', modelName: 'recorded-model',
  }, language);
  expect(page.getByRole('alert').textContent).toBe(locale.agentError.incompleteReasons.completion_report_rejected.title);
  expect(page.getByTestId('error-source').textContent).toContain('NomiFun');
  expect(page.getByTestId('error-source').textContent).toContain(locale.agentError.stages.completion_report);
  expect(page.getByTestId('error-source').textContent).toContain('recorded-model');
  expect(page.getByTestId('error-cause').textContent).toContain('Required SubAgent report was missing');
  expect(container.querySelector('.message-error-note__details')?.hasAttribute('hidden')).toBe(true);
});

test('missing recorded attribution remains unknown instead of blaming a provider', async () => {
  const { page } = await mount({ message: 'A preserved original diagnostic', code: 'AGENT_FAILURE_SOURCE_UNRECORDED', retryable: false });
  expect(page.getByTestId('error-source').textContent).toContain(enConversation.agentError.details.unavailable);
  expect(page.getByTestId('error-source').textContent).not.toContain('Model service');
  expect(page.getByTestId('error-cause').textContent).toContain('A preserved original diagnostic');
});

test.each(['en-US', 'zh-CN'])('local broker classification takes precedence over accompanying route diagnostics in %s', async language => {
  const locale = language === 'zh-CN' ? zhConversation : enConversation;
  for (const code of ['NOMIFUN_INTERNAL_ERROR', 'NOMIFUN_STATE_INCONSISTENT', 'NOMIFUN_TASK_INCOMPLETE'] as const) {
    const { page } = await mount({ code, message: 'The original local refusal', ownership: 'nomifun',
      taskIncompleteReason: code === 'NOMIFUN_TASK_INCOMPLETE' ? 'completion_report_rejected' : undefined,
      failureStage: code === 'NOMIFUN_TASK_INCOMPLETE' ? 'completion_report' : 'preparation',
      providerDiagnostic: { reason: 'configuration_error', providerId: 'original-route', modelName: 'recorded-model',
        requestId: 'original-request' },
    }, language);
    const expected = code === 'NOMIFUN_TASK_INCOMPLETE'
      ? locale.agentError.incompleteReasons.completion_report_rejected : locale.agentError.codes[code];
    expect(page.getByRole('alert').textContent).toBe(expected.title);
    expect(page.getByText(expected.body)).toBeTruthy();
    expect(page.queryByText(locale.agentError.providerReasons.configuration_error.body)).toBeNull();
    expect(page.getByTestId('error-source').textContent).toContain('NomiFun');
    expect(page.getByTestId('error-source').textContent).toContain('recorded-model');
    fireEvent.click(page.getByRole('button', { name: locale.agentError.expandDetails }));
    expect(page.getByRole('region').textContent).toContain('original-route');
    expect(page.getByRole('region').textContent).toContain('original-request');
    cleanup();
  }
});

test.each(['en-US', 'zh-CN'])('a provider parameter refusal exposes the original explanation without reclassifying prose in %s', async language => {
  const locale = language === 'zh-CN' ? zhConversation : enConversation;
  const { page } = await mount({ message: 'The model request failed', code: 'USER_LLM_PROVIDER_INVALID_REQUEST',
    ownership: 'user_llm_provider', failureStage: 'model_response',
    providerDiagnostic: { reason: 'invalid_request', providerParam: 'temperature',
      providerDetail: 'Parameter temperature is not supported. An example may mention key expired.',
    },
  }, language);
  expect(page.getByRole('alert').textContent).toBe(locale.agentError.providerReasons.invalid_request.title);
  expect(page.getByTestId('error-cause').textContent).toContain('Parameter temperature is not supported');
  expect(page.queryByText(locale.agentError.providerReasons.expired_key.title)).toBeNull();
});

test('a transport failure keeps its original cause ahead of a provider response detail', async () => {
  const { page } = await mount({ message: 'The model request failed', code: 'USER_LLM_PROVIDER_NETWORK_ERROR',
    providerDiagnostic: { reason: 'connection_failed', transportDetail: 'Connection closed before response',
      providerDetail: 'Provider response detail' },
  });
  expect(page.getByTestId('error-cause').textContent).toContain('Connection closed before response');
  expect(page.getByTestId('error-cause').textContent).not.toContain('Provider response detail');
});

test('a lifecycle error cannot add a second unclassified card beside the canonical failure', async () => {
  const { page } = await mount({ message: 'The exact cause', code: 'NOMIFUN_RUNTIME_FAILED', ownership: 'nomifun' });
  const status: IMessageAgentStatus = { id: 'status', type: 'agent_status',
    conversation_id: parseConversationId('0190f5fe-7c00-7a00-8000-000000000051'),
    position: 'center', created_at: 1234, content: { backend: 'nomi', status: 'error' } };
  const lifecycle = render(<MessageAgentStatus message={status} />);
  expect(lifecycle.container.childElementCount).toBe(0);
  expect(page.getAllByRole('alert')).toHaveLength(1);
});

test.each(['en-US', 'zh-CN'])('an HTTP contract rejection displays actionable configuration guidance in %s', async language => {
  const locale = language === 'zh-CN' ? zhConversation : enConversation;
  const detail = 'Conflict: AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED: capability agent.tool-discovery changed authority, effects, resources, dependencies or provenance';
  const error = new BackendHttpError({ method: 'POST', path: '/api/agent-sessions/session-1/turns', status: 409,
    body: { code: 'CONFLICT', error: detail } });
  const { page } = await mount(conversationRequestError(error, 'CONVERSATION_SEND_FAILED'), language);
  expect(page.getByRole('alert').textContent).toBe(locale.agentError.codes.AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED.title);
  expect(page.getByText(locale.agentError.codes.AGENT_SESSION_CONTRACT_EVOLUTION_REJECTED.body).closest('[hidden]')).toBeNull();
  expect(page.queryByText(locale.agentError.codes.CONVERSATION_SEND_FAILED.body)).toBeNull();
  expect(page.getByText(detail).closest('[hidden]')).toBeNull();
  fireEvent.click(page.getByRole('button', { name: locale.agentError.expandDetails }));
  expect(page.getByTestId('error-cause').textContent).toContain(detail);
});

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
    expect(result.page.getByText('An untrusted diagnostic says: key expired; auth rejected').closest('[hidden]')).toBeNull();
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
    code: 'USER_LLM_PROVIDER_AUTH_FAILED', ownership: 'user_llm_provider', failureStage: 'model_response',
    detail: 'safe original detail', modelName: 'primary-model', agentLabel: 'Test Agent',
    providerDiagnostic: { reason: 'expired_key', providerId: 'provider-2', modelName: 'failover-model', httpStatus: 401,
      endpoint: 'https://user:password-secret@api.example.com/v1?api_key=query-secret#fragment-secret',
      providerCode: 'key_expired', providerType: 'authentication_error', providerParam: 'headers.Authorization',
      requestId: 'req-123', protocol: 'openai_chat', authScheme: 'bearer', retryAfterMs: 0, contentType: 'application/json',
      transportDetail: 'connection failed at https://user:password-secret@api.example.com/v1?token=query-secret',
    } };
  const { page, container } = await mount(error);
  expect(page.getByTestId('error-source').textContent).toContain('Model service');
  expect(page.getByTestId('error-source').textContent).toContain('failover-model');
  expect(page.getByTestId('error-source').textContent).not.toContain('primary-model');
  expect(page.getByTestId('error-cause').textContent).not.toContain('password-secret');
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

test.each(['en-US', 'zh-CN'])('native authentication and unknown results use canonical classification and show exact facts in %s', async language => {
  const locale = language === 'zh-CN' ? zhConversation : enConversation;
  for (const code of ['SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED', 'SYSTEM_READ_RESULT_UNKNOWN'] as const) {
    const cancelled = code === 'SYSTEM_EXECUTION_AUTHENTICATION_CANCELLED';
    const { page } = await mount({ code, ownership: 'nomifun', message:'PRESET_RESOURCE_NOT_BOUND and access denied are only prose',
      systemExecutionError: { code, message:'original native owner message', kind:cancelled ? 'Uncategorized' : null,
        raw_os_error:cancelled ? 1223 : null, user_code_not_started:cancelled, cleanup_proven:cancelled } }, language);
    expect(page.getByRole('alert').textContent).toBe(locale.agentError.codes[code].title);
    expect(page.getByTestId('error-source').textContent).toContain(locale.agentError.sources.operating_system);
    expect(page.queryByText(locale.agentError.codes.NOMIFUN_INTERNAL_ERROR.title)).toBeNull();
    fireEvent.click(page.getByRole('button', { name:locale.agentError.expandDetails }));
    const text = page.getByRole('region').textContent;
    expect(text).toContain(code);
    expect(page.getByTestId('error-cause').textContent).toContain('original native owner message');
    expect(text).toContain(locale.agentError.details.userCodeNotStarted);
    expect(text).toContain(cancelled ? '1223' : locale.agentError.details.unavailable);
    expect(text).toContain(cancelled ? 'true' : 'false');
    cleanup();
  }
});

test.each(['en-US', 'zh-CN'])('typed resource preparation failure identifies the local phase and shows missing grants in %s', async language => {
  const locale = language === 'zh-CN' ? zhConversation : enConversation;
  const { page } = await mount({ code: 'NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE', message: 'Local admission failed', retryable: false,
    resourceBindingFailure: { missing_resource_kinds: ['workspace'], missing_resource_operations: { workspace: ['write'] } },
    providerDiagnostic: { reason: 'auth_failed' },
  }, language);
  expect(page.getByRole('alert').textContent).toBe(locale.agentError.codes.NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE.title);
  expect(page.getByText(locale.agentError.codes.NOMIFUN_AGENT_RESOURCE_BINDING_INCOMPLETE.body).closest('[hidden]')).toBeNull();
  fireEvent.click(page.getByRole('button', { name: locale.agentError.expandDetails }));
  expect(page.getByRole('region').textContent).toContain('workspace');
  expect(page.getByRole('region').textContent).toContain('write');
  expect(page.queryByText(locale.agentError.providerReasons.auth_failed.body)).toBeNull();
});
