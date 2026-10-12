import '../../../../../test/setup-dom.ts';

import { act, cleanup, fireEvent, render, renderHook } from '@testing-library/react';
import { Message } from '@arco-design/web-react';
import { afterEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useRef, type ReactNode } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import type { ApplyAgentSessionSwitchResponse, OfficialPresetTemplate, PreviewAgentSessionSwitchResponse } from '@/common/types/agentPlatform';
import { parseConversationId } from '@/common/types/ids';
import conversation from '@/renderer/services/i18n/locales/en-US/conversation.json';
import agentSettings from '@/renderer/services/i18n/locales/en-US/agentSettings.json';
import guid from '@/renderer/services/i18n/locales/en-US/guid.json';
import GuidAgentSelector from '@/renderer/pages/guid/components/GuidAgentSelector';
import type { GuidAgentSelection } from '@/renderer/pages/guid/types';
import { useConversationAgentSwitch } from './useConversationAgentSwitch';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'en-US', resources: { 'en-US': { translation: { conversation, agentSettings, guid } } },
  interpolation: { escapeValue: false },
});
const wrapper = ({ children }: { children: ReactNode }) => <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
const firstSession = parseConversationId('0190f5fe-7c00-7a00-8000-000000000101');
const secondSession = parseConversationId('0190f5fe-7c00-7a00-8000-000000000102');
const minimal: GuidAgentSelection = { kind: 'template', templateKey: 'chat.minimal' };
const general: GuidAgentSelection = { kind: 'template', templateKey: 'assistant.general' };
const coding: GuidAgentSelection = { kind: 'template', templateKey: 'coding.codex' };
const preview = {
  can_apply: true, expected_binding_version: 4,
  target: { resolved_snapshot_ref: { snapshot_id: 'target-snapshot', snapshot_digest: 'b'.repeat(64) } },
  // Resource differences are informational. The renderer adds no asset gate.
  resources: { retained: [], dropped: [{ resource_kind: 'workspace', resource_id: 'previous' }], missing_kinds: ['knowledge_base'] },
  handoff: { available: true }, blockers: [],
} as unknown as PreviewAgentSessionSwitchResponse;
const applied = { current_agent_label: 'General' } as ApplyAgentSessionSwitchResponse;
const deferred = <T,>() => {
  let resolve!: (value: T) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<T>((res, rej) => { resolve = res; reject = rej; });
  return { promise, resolve, reject };
};
type Operations = NonNullable<Parameters<typeof useConversationAgentSwitch>[1]>;
const operations = (overrides: Partial<Operations> = {}) => ({
  preview: mock(async () => preview),
  apply: mock(async () => applied),
  refresh: mock(async () => {}),
  ...overrides,
});
const mount = (ports: Operations, processing = false) => renderHook(({ sessionId, selection }) => {
  const switchingRef = useRef(false);
  const configurationChangingRef = useRef(false);
  const reasoningUpdatingRef = useRef(false);
  return useConversationAgentSwitch({ sessionId, currentSelection: selection, processing, switchingRef, configurationChangingRef, reasoningUpdatingRef }, ports);
}, { initialProps: { sessionId: firstSession, selection: minimal }, wrapper });

afterEach(() => { cleanup(); mock.restore(); });

describe('current conversation Agent switching', () => {
  test('selecting an Agent starts the switch immediately and keeps the input draft mounted', async () => {
    const pending = deferred<PreviewAgentSessionSwitchResponse>();
    const ports = operations({ preview: mock(() => pending.promise) });
    spyOn(Message, 'success').mockImplementation(() => () => {});
    let completing!: Promise<void>;
    const Harness = () => {
      const switchingRef = useRef(false);
      const configurationChangingRef = useRef(false);
      const reasoningUpdatingRef = useRef(false);
      const switcher = useConversationAgentSwitch({
        sessionId: firstSession, currentSelection: minimal, processing: false,
        switchingRef, configurationChangingRef, reasoningUpdatingRef,
      }, ports);
      return <>
        <GuidAgentSelector presets={[]} selection={minimal}
          officialTemplates={['chat.minimal', 'assistant.general'].map((template_key) => ({ template_key }) as OfficialPresetTemplate)}
          switching={switcher.switching} switchingLabel={conversation.chat.agentSwitch.loading}
          onSelectPreset={() => {}} onSelectTemplate={(templateKey) => {
            completing = switcher.switchAgent({ kind: 'template', templateKey });
          }} />
        <textarea aria-label='Message draft' defaultValue='Keep this draft' />
      </>;
    };
    const page = render(<I18nextProvider i18n={i18n}><MemoryRouter><Harness /></MemoryRouter></I18nextProvider>);
    const draft = page.getByRole('textbox', { name: 'Message draft' }) as HTMLTextAreaElement;
    await act(async () => { fireEvent.click(page.getByTestId('guid-agent-selector')); });
    act(() => { fireEvent.click(page.getByRole('button', { name: agentSettings.template.assistant.general.name })); });
    expect(ports.preview).toHaveBeenCalledTimes(1);
    expect(page.queryByRole('dialog')).toBeNull();
    expect(page.getByTestId('guid-agent-selector').getAttribute('aria-busy')).toBe('true');
    expect(page.getByRole('textbox', { name: 'Message draft' })).toBe(draft);
    expect(draft.value).toBe('Keep this draft');
    await act(async () => { pending.resolve(preview); await completing; });
    expect(ports.apply).toHaveBeenCalledTimes(1);
    expect(page.getByTestId('guid-agent-selector').hasAttribute('aria-busy')).toBe(false);
    expect(page.getByRole('textbox', { name: 'Message draft' })).toBe(draft);
    expect(draft.value).toBe('Keep this draft');
  });

  test('applies one selection directly, retaining binding safety and automatic handoff', async () => {
    const pending = deferred<PreviewAgentSessionSwitchResponse>();
    const ports = operations({ preview: mock(() => pending.promise) });
    const success = spyOn(Message, 'success').mockImplementation(() => () => {});
    const hook = mount(ports);
    let switching!: Promise<void>;
    act(() => {
      switching = hook.result.current.switchAgent(general);
      void hook.result.current.switchAgent(coding);
    });
    expect(hook.result.current.switching).toBe(true);
    expect(ports.preview).toHaveBeenCalledTimes(1);
    expect(ports.apply).toHaveBeenCalledTimes(0);
    await act(async () => { pending.resolve(preview); await switching; });
    expect(ports.apply).toHaveBeenCalledTimes(1);
    const [sessionId, request, idempotencyKey] = (ports.apply as ReturnType<typeof mock>).mock.calls[0];
    expect(sessionId).toBe(firstSession);
    expect(request).toEqual({
      selection: { kind: 'template', template_key: 'assistant.general' },
      handoff_mode: 'continue_task', expected_binding_version: 4,
      expected_resolved_snapshot_ref: preview.target.resolved_snapshot_ref,
    });
    expect(idempotencyKey).toMatch(/^[\da-f-]{36}$/);
    expect(ports.refresh).toHaveBeenCalledWith(firstSession);
    expect(hook.result.current.switching).toBe(false);
    expect(success).toHaveBeenCalledTimes(1);
  });

  test('uses chat context when structured handoff is unavailable', async () => {
    const ports = operations({ preview: mock(async () => ({ ...preview, handoff: { ...preview.handoff, available: false } })) });
    spyOn(Message, 'success').mockImplementation(() => () => {});
    const hook = mount(ports);
    await act(async () => { await hook.result.current.switchAgent(general); });
    expect((ports.apply as ReturnType<typeof mock>).mock.calls[0][1].handoff_mode).toBe('context_only');
  });

  test('stops a late preview from changing another conversation or its pending selection', async () => {
    const firstPreview = deferred<PreviewAgentSessionSwitchResponse>();
    const secondPreview = deferred<PreviewAgentSessionSwitchResponse>();
    const ports = operations({ preview: mock((sessionId) => sessionId === firstSession ? firstPreview.promise : secondPreview.promise) });
    const success = spyOn(Message, 'success').mockImplementation(() => () => {});
    const hook = mount(ports);
    let firstSwitch!: Promise<void>;
    act(() => { firstSwitch = hook.result.current.switchAgent(general); });
    hook.rerender({ sessionId: secondSession, selection: minimal });
    expect(hook.result.current.switching).toBe(false);
    let secondSwitch!: Promise<void>;
    act(() => { secondSwitch = hook.result.current.switchAgent(coding); });
    await act(async () => { firstPreview.resolve(preview); await firstSwitch; });
    expect(ports.apply).toHaveBeenCalledTimes(0);
    expect(hook.result.current.switching).toBe(true);
    await act(async () => { secondPreview.resolve(preview); await secondSwitch; });
    expect(ports.apply).toHaveBeenCalledTimes(1);
    expect((ports.apply as ReturnType<typeof mock>).mock.calls[0][0]).toBe(secondSession);
    expect(success).toHaveBeenCalledTimes(1);
  });

  test('retains the active-turn boundary and surfaces backend safety blockers briefly', async () => {
    const warning = spyOn(Message, 'warning').mockImplementation(() => () => {});
    const ports = operations();
    const running = mount(ports, true);
    await act(async () => { await running.result.current.switchAgent(general); });
    expect(ports.preview).toHaveBeenCalledTimes(0);
    expect(warning).toHaveBeenCalledWith(conversation.chat.agentSwitch.waitForTurn);
    running.unmount();
    const blockedPorts = operations({ preview: mock(async () => ({
      ...preview, can_apply: false,
      blockers: [{ code: 'AGENT_SESSION_EFFECTS_UNSETTLED', message: 'Effects pending' }],
    })) });
    const blocked = mount(blockedPorts);
    await act(async () => { await blocked.result.current.switchAgent(general); });
    expect(blockedPorts.apply).toHaveBeenCalledTimes(0);
    expect(blocked.result.current.switching).toBe(false);
    expect(warning).toHaveBeenLastCalledWith(expect.stringContaining('Settle outstanding effects'));
  });

  test('invalidates an old preview even after navigating back to the same conversation', async () => {
    const pending = deferred<PreviewAgentSessionSwitchResponse>();
    const ports = operations({ preview: mock(() => pending.promise) });
    const hook = mount(ports);
    let switching!: Promise<void>;
    act(() => { switching = hook.result.current.switchAgent(general); });
    hook.rerender({ sessionId: secondSession, selection: minimal });
    hook.rerender({ sessionId: firstSession, selection: minimal });
    expect(hook.result.current.switching).toBe(false);
    await act(async () => { pending.resolve(preview); await switching; });
    expect(ports.apply).toHaveBeenCalledTimes(0);
  });

  test('allows retry after failure and avoids reporting a committed switch as failed on refresh errors', async () => {
    const pending = deferred<ApplyAgentSessionSwitchResponse>();
    const ports = operations({ apply: mock(() => pending.promise) });
    const error = spyOn(Message, 'error').mockImplementation(() => () => {});
    const success = spyOn(Message, 'success').mockImplementation(() => () => {});
    spyOn(console, 'error').mockImplementation(() => {});
    const hook = mount(ports);
    let switching!: Promise<void>;
    await act(async () => { switching = hook.result.current.switchAgent(general); });
    await act(async () => { pending.reject(new Error('Network unavailable')); await switching; });
    expect(hook.result.current.switching).toBe(false);
    expect(error).toHaveBeenCalledTimes(1);
    ports.apply = mock(async () => applied);
    ports.refresh = mock(async () => { throw new Error('Cache unavailable'); });
    await act(async () => { await hook.result.current.switchAgent(general); });
    expect(error).toHaveBeenCalledTimes(1);
    expect(success).toHaveBeenCalledTimes(1);
    expect(hook.result.current.switching).toBe(false);
  });

  test('never applies a preview after the selector has unmounted', async () => {
    const pending = deferred<PreviewAgentSessionSwitchResponse>();
    const ports = operations({ preview: mock(() => pending.promise) });
    const hook = mount(ports);
    let switching!: Promise<void>;
    act(() => { switching = hook.result.current.switchAgent(general); });
    hook.unmount();
    pending.resolve(preview);
    await switching;
    expect(ports.apply).toHaveBeenCalledTimes(0);
  });

  test('waits for model and reasoning changes and skips the already selected Agent', async () => {
    const ports = operations();
    spyOn(Message, 'success').mockImplementation(() => () => {});
    const hook = renderHook(() => {
      const switchingRef = useRef(false);
      const configurationChangingRef = useRef(true);
      const reasoningUpdatingRef = useRef(false);
      const switcher = useConversationAgentSwitch({
        sessionId: firstSession, currentSelection: minimal, processing: false,
        switchingRef, configurationChangingRef, reasoningUpdatingRef,
      }, ports);
      return { ...switcher, configurationChangingRef, reasoningUpdatingRef };
    }, { wrapper });
    await act(async () => { await hook.result.current.switchAgent(general); });
    expect(ports.preview).toHaveBeenCalledTimes(0);
    hook.result.current.configurationChangingRef.current = false;
    hook.result.current.reasoningUpdatingRef.current = true;
    await act(async () => { await hook.result.current.switchAgent(general); });
    expect(ports.preview).toHaveBeenCalledTimes(0);
    hook.result.current.reasoningUpdatingRef.current = false;
    await act(async () => { await hook.result.current.switchAgent(minimal); });
    expect(ports.preview).toHaveBeenCalledTimes(0);
    await act(async () => { await hook.result.current.switchAgent(general); });
    expect(ports.apply).toHaveBeenCalledTimes(1);
  });
});
