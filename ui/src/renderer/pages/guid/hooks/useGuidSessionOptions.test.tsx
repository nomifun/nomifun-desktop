import '../../../../../test/setup-dom.ts';

import { Message } from '@arco-design/web-react';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, expect, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import type { PropsWithChildren } from 'react';
import { I18nextProvider } from 'react-i18next';

import { ipcBridge } from '@/common';
import { parseConversationId } from '@/common/types/ids';
import { createDefaultIdmmConfig } from '@/common/types/idmm';
import { useGuidSessionOptions } from './useGuidSessionOptions';

const i18n = createInstance();
await i18n.init({ lng: 'en', resources: { en: { translation: {} } } });

const wrapper = ({ children }: PropsWithChildren) => (
  <I18nextProvider i18n={i18n}>{children}</I18nextProvider>
);

afterEach(() => cleanup());

test('Agent IDMM defaults rely on server inheritance while an explicit Session override is applied', async () => {
  const setConfig = spyOn(ipcBridge.idmm.setConfig, 'invoke').mockResolvedValue(
    undefined as never
  );
  const hook = renderHook(() => useGuidSessionOptions(), { wrapper });
  const conversationId = parseConversationId('0190f5fe-7c00-7a00-8000-000000000018');

  act(() =>
    hook.result.current.setIdmmDefault({
      ...createDefaultIdmmConfig(),
      mode: 'rule_only',
    })
  );
  await hook.result.current.applyToConversation(conversationId);
  expect(setConfig).not.toHaveBeenCalled();

  act(() => hook.result.current.setIdmm(createDefaultIdmmConfig()));
  await hook.result.current.applyToConversation(conversationId);
  expect(setConfig).toHaveBeenCalledTimes(1);
  expect(setConfig.mock.calls[0]?.[0]).toEqual({
    agent_session_id: conversationId,
    config: createDefaultIdmmConfig(),
  });
  setConfig.mockRestore();
});

test('AutoWork launch configuration failure propagates to the Guid session owner', async () => {
  const failure = new Error('frozen AgentSession workspace is unavailable');
  const setAutoWork = spyOn(
    ipcBridge.requirements.setAutoWork,
    'invoke'
  ).mockRejectedValue(failure);
  const warning = spyOn(Message, 'warning').mockImplementation(() => undefined as never);
  const consoleError = spyOn(console, 'error').mockImplementation(() => {});
  const hook = renderHook(() => useGuidSessionOptions(), { wrapper });

  act(() => hook.result.current.setAutoWork({ enabled: true, tag: 'release' }));
  await expect(
    hook.result.current.applyToConversation(
      parseConversationId('0190f5fe-7c00-7a00-8000-000000000017')
    )
  ).rejects.toBe(failure);

  expect(setAutoWork).toHaveBeenCalledTimes(1);
  expect(warning).toHaveBeenCalledTimes(1);
  setAutoWork.mockRestore();
  warning.mockRestore();
  consoleError.mockRestore();
});
