import '../../../../test/setup-dom.ts';
import { afterEach, describe, expect, test } from 'bun:test';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import { SWRConfig } from 'swr';
import type { RuntimeEngineSelection } from '@/common/types/agentPlatform';
import en from '../../services/i18n/locales/en-US/agentSettings.json';
import AgentRuntimeEngineSelector, { runtimeEngineKey, runtimeEngineOptions } from './AgentRuntimeEngineSelector';

const i18n = createInstance();
await i18n.use(initReactI18next).init({ lng: 'en-US', resources: { 'en-US': { translation: { agentSettings: en } } } });
afterEach(cleanup);

const descriptor = {
  family_id: 'customer.workflow', build_id: 'build-42', build_digest: 'a'.repeat(64),
  host_contract_version: 1, display_name: 'Custom runtime', supported_profiles: ['review', 'workflow'],
};

const wrap = (child: React.ReactNode, catalog = [descriptor]) => (
  <I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), fallback: { 'runtime-engines': catalog }, revalidateOnMount: false }}>
    {child}
  </SWRConfig></I18nextProvider>
);

describe('runtime engine catalog options', () => {
  test('the workbench selection changes the controlled Agent draft and can reset to default', async () => {
    const Harness = () => {
      const [value, setValue] = useState<RuntimeEngineSelection>();
      return <><AgentRuntimeEngineSelector value={value} onChange={setValue} /><output data-testid='engine'>{JSON.stringify(value ?? null)}</output></>;
    };
    const screen = render(wrap(<Harness />));
    const select = screen.getByRole('combobox', { name: en.runtimeEngine.label });
    fireEvent.click(select);
    fireEvent.click(await screen.findByText('Custom runtime · review · build-42'));
    await waitFor(() => expect(JSON.parse(screen.getByTestId('engine').textContent!)).toEqual(runtimeEngineOptions([descriptor])[0].selection));
    fireEvent.click(select);
    fireEvent.click(await screen.findByText(en.runtimeEngine.default));
    await waitFor(() => expect(screen.getByTestId('engine').textContent).toBe('null'));
  });

  test('does not silently replace a saved but unavailable build', () => {
    const selection = runtimeEngineOptions([descriptor])[0].selection;
    let changed = false;
    const screen = render(wrap(<AgentRuntimeEngineSelector value={selection} onChange={() => { changed = true; }} />, []));
    expect(screen.getAllByText(en.runtimeEngine.unavailable).length).toBeGreaterThan(0);
    expect(changed).toBe(false);
  });

  test('preserves a configured channel without claiming it is an absent build', () => {
    const value: RuntimeEngineSelection = { selector: { selection: 'channel', family_id: descriptor.family_id, channel: 'stable' }, profile: 'workflow' };
    const screen = render(wrap(<AgentRuntimeEngineSelector value={value} onChange={() => {}} />));
    expect(screen.getByText('Custom runtime · workflow · stable')).toBeTruthy();
    expect(screen.queryByText(en.runtimeEngine.unavailable)).toBeNull();
  });
  test('discovers arbitrary families and profiles with exact build identity', () => {
    const descriptor = {
      family_id: 'customer.workflow', build_id: 'build-42', build_digest: 'a'.repeat(64),
      host_contract_version: 1, display_name: 'Custom runtime', supported_profiles: ['review', 'workflow'],
    };
    const options = runtimeEngineOptions([descriptor]);
    expect(options.map((option) => option.selection.profile)).toEqual(['review', 'workflow']);
    expect(options[0].selection.selector).toEqual({
      selection: 'exact', family_id: descriptor.family_id, build_id: descriptor.build_id, build_digest: descriptor.build_digest,
    });
    expect(new Set(options.map((option) => option.value)).size).toBe(2);
    expect(runtimeEngineOptions([])).toEqual([]);
    // Persisted Rust JSON may return fields in a different order.
    expect(runtimeEngineKey({ profile: 'review', selector: {
      build_digest: descriptor.build_digest, build_id: descriptor.build_id,
      family_id: descriptor.family_id, selection: 'exact',
    } })).toBe(options[0].value);
  });
});
