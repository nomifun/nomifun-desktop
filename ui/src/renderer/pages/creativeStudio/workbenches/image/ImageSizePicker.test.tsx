/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';
import { act, cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { useState } from 'react';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import zh from '../../../../services/i18n/locales/zh-CN/creativeStudio.json';
import en from '../../../../services/i18n/locales/en-US/creativeStudio.json';
import ImageSizePicker from './ImageSizePicker';
import { imageWorkbenchSizePolicyForModel } from './types';

const i18n = createInstance();
await i18n.use(initReactI18next).init({
  lng: 'zh-CN',
  resources: {
    'zh-CN': { translation: { creativeStudio: zh } },
    'en-US': { translation: { creativeStudio: en } },
  },
});
afterEach(() => cleanup());
const options = imageWorkbenchSizePolicyForModel(null).options;

describe('ImageSizePicker interactions', () => {
  test('updates resolution independently and retains it when switching supported ratios', () => {
    const selections: string[] = [];
    const Harness = () => {
      const [value, setValue] = useState('16:9');
      return <ImageSizePicker options={options} value={value} onChange={(option) => {
        selections.push(option.value);
        setValue(option.value);
      }} />;
    };
    const view = render(<I18nextProvider i18n={i18n}><Harness /></I18nextProvider>);
    const ratios = within(view.getByRole('group', { name: '宽高比' }));
    const resolutions = within(view.getByRole('group', { name: '分辨率' }));
    expect(ratios.getAllByRole('button')).toHaveLength(9);
    fireEvent.click(resolutions.getByRole('button', { name: '4K' }));
    expect(ratios.getByRole('button', { name: '16:9', pressed: true })).not.toBeNull();
    expect(view.getByText('3840 × 2160')).not.toBeNull();
    fireEvent.click(ratios.getByRole('button', { name: '9:16' }));
    expect(resolutions.getByRole('button', { name: '4K', pressed: true })).not.toBeNull();
    expect(view.getByText('2160 × 3840')).not.toBeNull();
    fireEvent.click(ratios.getByRole('button', { name: '1:1' }));
    expect(resolutions.queryByRole('button', { name: '4K' })).toBeNull();
    expect(view.getByText('1024 × 1024')).not.toBeNull();
    fireEvent.click(ratios.getByRole('button', { name: '自动' }));
    expect(resolutions.getAllByRole('button')).toHaveLength(1);
    expect(resolutions.getByRole('button', { name: '自动', pressed: true })).not.toBeNull();
    expect(selections).toEqual(['3840x2160', '2160x3840', '1:1', 'auto']);
  });

  test('supports disabled controls and policy changes without stale higher resolutions', () => {
    const selections: string[] = [];
    const onChange = (option: { value: string }) => selections.push(option.value);
    const view = render(<I18nextProvider i18n={i18n}>
      <ImageSizePicker options={options} value='2048x2048' disabled onChange={onChange} />
    </I18nextProvider>);
    fireEvent.click(view.getByRole('button', { name: '16:9' }));
    expect(selections).toEqual([]);
    const restricted = imageWorkbenchSizePolicyForModel({ model: 'step-image-edit-2', protocol: 'stepfun.images' });
    view.rerender(<I18nextProvider i18n={i18n}>
      <ImageSizePicker options={restricted.options} value='2048x2048' onChange={onChange} />
    </I18nextProvider>);
    expect(view.queryByRole('button', { name: '2K' })).toBeNull();
    expect(view.getByRole('button', { name: '1:1', pressed: true })).not.toBeNull();
    fireEvent.click(view.getByRole('button', { name: '16:9' }));
    expect(selections).toEqual(['16:9']);
  });

  test('renders English group labels and automatic sizing', async () => {
    await act(async () => { await i18n.changeLanguage('en-US'); });
    try {
      const view = render(<I18nextProvider i18n={i18n}>
        <ImageSizePicker options={options} value='auto' onChange={() => undefined} />
      </I18nextProvider>);
      expect(view.getByRole('group', { name: 'Aspect ratio' })).not.toBeNull();
      expect(view.getByRole('group', { name: 'Resolution' })).not.toBeNull();
      expect(view.getAllByRole('button', { name: 'Auto', pressed: true })).toHaveLength(2);
    } finally {
      await act(async () => { await i18n.changeLanguage('zh-CN'); });
    }
  });
});
