import '../../../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, beforeEach, expect, test } from 'bun:test';
import { uuidv7 } from '@/common/utils';
import { agentBrowserStorageGenerationKey, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { useGuidDraftState } from './useGuidDraftState';

beforeEach(() => { setBrowserStorageGeneration(uuidv7()); sessionStorage.clear(); });
afterEach(() => { cleanup(); sessionStorage.clear(); });

test('reapplying the same application workspace persists it without rendering its composer again', () => {
  const workspace = 'C:/NomiFun/apps/draft-application';
  const scope = 'app-workspace:draft-application:new';
  const storageKey = agentBrowserStorageGenerationKey(`guid-draft:${scope}:workspace`);
  let renders = 0;
  const hook = renderHook(() => {
    renders++;
    return useGuidDraftState('workspace', workspace, scope);
  });
  const initialRenders = renders;
  act(() => hook.result.current[1](workspace));
  expect(renders).toBe(initialRenders);
  expect(sessionStorage.getItem(storageKey)).toBe(JSON.stringify(workspace));

  // Reapplying a draft still repairs missing browser storage while the
  // composer remains settled.
  sessionStorage.removeItem(storageKey);
  act(() => hook.result.current[1](current => current));
  expect(renders).toBe(initialRenders);
  expect(sessionStorage.getItem(storageKey)).toBe(JSON.stringify(workspace));
});

test('functional application draft updates use the latest value while repeated equal updates settle', () => {
  let renders = 0;
  const hook = renderHook(() => {
    renders++;
    return useGuidDraftState('input', '', 'app-workspace:functional:new');
  });
  act(() => {
    hook.result.current[1](current => `${current}First`);
    hook.result.current[1](current => `${current} second`);
  });
  expect(hook.result.current[0]).toBe('First second');
  const settledRenders = renders;
  act(() => hook.result.current[1](current => current));
  expect(hook.result.current[0]).toBe('First second');
  expect(renders).toBe(settledRenders);
});

test('late application draft cleanup writes its original scope without replacing the newly opened application', () => {
  const firstScope = 'app-workspace:first:new';
  const secondScope = 'app-workspace:second:new';
  const hook = renderHook(({ scope }) => useGuidDraftState('input', '', scope), { initialProps: { scope: firstScope } });
  act(() => hook.result.current[1]('First application draft'));
  const lateFirstUpdate = hook.result.current[1];
  hook.rerender({ scope: secondScope });
  expect(hook.result.current[0]).toBe('');
  act(() => hook.result.current[1]('Second application draft'));
  act(() => lateFirstUpdate(current => `${current} completed`));
  expect(hook.result.current[0]).toBe('Second application draft');
  expect(sessionStorage.getItem(agentBrowserStorageGenerationKey(`guid-draft:${firstScope}:input`))).toBe(JSON.stringify('First application draft completed'));
  expect(sessionStorage.getItem(agentBrowserStorageGenerationKey(`guid-draft:${secondScope}:input`))).toBe(JSON.stringify('Second application draft'));

  const lateSecondCleanup = hook.result.current[1];
  hook.unmount();
  act(() => lateSecondCleanup(''));
  const reopened = renderHook(() => useGuidDraftState('input', 'Fallback input', secondScope));
  expect(reopened.result.current[0]).toBe('');
});
