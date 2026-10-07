import '../../../../test/setup-dom.ts';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { useState } from 'react';
import AgentEditorTabs from './AgentEditorTabs';

afterEach(cleanup);

test('editor tabs use roving focus with Arrow, Home and End keys', async () => {
  const tabs = [
    { key: 'modules', label: 'Modules' },
    { key: 'providers', label: 'Providers' },
    { key: 'settings', label: 'Settings' },
  ];
  const Harness = () => {
    const [active, setActive] = useState('modules');
    return <AgentEditorTabs tabs={tabs} active={active} onChange={setActive} idPrefix='test' label='Editor' />;
  };
  const screen = render(<Harness />);
  const modules = screen.getByRole('tab', { name: 'Modules' });
  const providers = screen.getByRole('tab', { name: 'Providers' });
  const settings = screen.getByRole('tab', { name: 'Settings' });
  modules.focus();
  fireEvent.keyDown(modules, { key: 'ArrowRight' });
  await waitFor(() => expect(providers.getAttribute('aria-selected')).toBe('true'));
  expect(document.activeElement).toBe(providers);
  fireEvent.keyDown(providers, { key: 'End' });
  await waitFor(() => expect(settings.getAttribute('aria-selected')).toBe('true'));
  expect(document.activeElement).toBe(settings);
  fireEvent.keyDown(settings, { key: 'Home' });
  await waitFor(() => expect(modules.getAttribute('aria-selected')).toBe('true'));
  expect(document.activeElement).toBe(modules);
  expect(modules.tabIndex).toBe(0);
  expect(providers.tabIndex).toBe(-1);
});
