import '../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render } from '@testing-library/react';
import { afterEach, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import zhSettings from '@/renderer/services/i18n/locales/zh-CN/settings.json';
import { ContextLimitSelect } from './ContextLimitSelect';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN',
  resources: { 'zh-CN': { translation: { settings: zhSettings } } },
});

afterEach(cleanup);

const Harness = () => {
  const [value, setValue] = useState<number>();
  return (
    <I18nextProvider i18n={i18n}>
      <ContextLimitSelect value={value} onChange={setValue} />
      <output data-testid='saved-context'>{value ?? 'default'}</output>
    </I18nextProvider>
  );
};

test('custom context entry saves a token count and can return to runtime default', () => {
  const page = render(<Harness />);
  fireEvent.click(page.getByRole('combobox'));
  fireEvent.click(page.getByRole('option', { name: '自定义' }));
  const input = page.getByRole('spinbutton', { name: '输入 tokens 数量' });
  fireEvent.change(input, { target: { value: '65536' } });
  fireEvent.blur(input);
  expect(page.getByTestId('saved-context').textContent).toBe('65536');

  fireEvent.click(page.getByRole('combobox'));
  fireEvent.click(page.getByRole('option', { name: '自动（运行时默认）' }));
  expect(page.getByTestId('saved-context').textContent).toBe('default');
});
