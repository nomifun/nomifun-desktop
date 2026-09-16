import '../../../../../test/setup-dom.ts';
import { useState } from 'react';
import { afterEach, expect, test } from 'bun:test';
import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { StopButtonHostContext, StopButtonPortal } from './StopButtonPortal';

afterEach(cleanup);

test('focus mode moves the same stop owner out of hidden chat and keeps pending state when returning', () => {
  const host = document.createElement('div');
  document.body.appendChild(host);
  let calls = 0;
  function Composer() {
    const [pending, setPending] = useState(false);
    return <StopButtonPortal><button disabled={pending} onClick={() => { calls++; setPending(true); }}>{pending ? 'Stopping' : 'Stop'}</button></StopButtonPortal>;
  }
  const view = (focused: boolean) => <StopButtonHostContext.Provider value={focused ? host : null}><div style={{ display: focused ? 'none' : 'block' }}><Composer /></div></StopButtonHostContext.Provider>;
  const rendered = render(view(false));
  try {
    expect(within(rendered.container).getByRole('button', { name: 'Stop' })).toBeTruthy();
    rendered.rerender(view(true));
    expect(rendered.container.querySelector('button')).toBeNull();
    fireEvent.click(within(host).getByRole('button', { name: 'Stop' }));
    expect(calls).toBe(1);
    rendered.rerender(view(false));
    expect(host.childElementCount).toBe(0);
    const pending = within(rendered.container).getByRole('button', { name: 'Stopping' }) as HTMLButtonElement;
    expect(pending.disabled).toBe(true);
    fireEvent.click(pending);
    expect(calls).toBe(1);
    rendered.unmount();
    expect(host.childElementCount).toBe(0);
  } finally { host.remove(); }
});
