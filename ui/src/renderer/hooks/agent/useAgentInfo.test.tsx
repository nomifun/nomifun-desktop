import '../../../../test/setup-dom.ts';
import { act, cleanup, renderHook } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider, initReactI18next } from 'react-i18next';
import type { ReactNode } from 'react';
import type { TChatConversation } from '@/common/config/storage';
import en from '@/renderer/services/i18n/locales/en-US/agentSettings.json';
import zh from '@/renderer/services/i18n/locales/zh-CN/agentSettings.json';
import { useAgentInfo } from './useAgentInfo';

afterEach(cleanup);

const conversation = (extra: Record<string, unknown>, name: string): TChatConversation => ({
  id: '0190f5fe-7c00-7a00-8000-000000000102', type: 'nomi', name: 'Image conversation',
  preset_id: '0190f5fe-7c00-7a00-8000-000000000104', extra,
  agent_snapshot: { preset_id: '0190f5fe-7c00-7a00-8000-000000000104', preset_name: name, preset_revision: 1 },
}) as unknown as TChatConversation;

async function mount(value: TChatConversation) {
  const i18n = createInstance();
  await i18n.use(initReactI18next).init({
    lng: 'zh-CN', resources: {
      'zh-CN': { translation: { agentSettings: zh } },
      'en-US': { translation: { agentSettings: en } },
    },
  });
  const wrapper = ({ children }: { children: ReactNode }) => <I18nextProvider i18n={i18n}>{children}</I18nextProvider>;
  return { ...renderHook(() => useAgentInfo(value), { wrapper }), i18n };
}

describe('official creation Agent presentation', () => {
  test('old snapshot and projection names cannot override the verified product identity', async () => {
    for (const oldName of [['多', '模'].join(''), ['创意', '工坊'].join(''), 'creative-studio.default']) {
      const hook = await mount(conversation({ official_template_key: 'creative-studio.default', agent_name: oldName }, oldName));
      expect(hook.result.current.info?.name).toBe('创作');
      await act(async () => { await hook.i18n.changeLanguage('en-US'); });
      expect(hook.result.current.info?.name).toBe('Creation');
      hook.unmount();
    }
  });

  test('a personal Agent with a similar name retains its frozen identity', async () => {
    const name = ['创意', '工坊'].join('');
    const hook = await mount(conversation({ agent_name: name }, name));
    expect(hook.result.current.info?.name).toBe(name);
  });
});

test('product-owned official Agents use the same locale-aware identity as ordinary chat', async () => {
  for (const [key, chinese, english] of [
    ['companion.default', '伙伴', 'Companion'],
    ['customer-service.default', '客户服务', 'Customer Service'],
  ]) {
    const hook = await mount(conversation({ official_template_key: key, agent_name: key }, key));
    expect(hook.result.current.info?.name).toBe(chinese);
    await act(async () => { await hook.i18n.changeLanguage('en-US'); });
    expect(hook.result.current.info?.name).toBe(english);
    hook.unmount();
  }
});

test('a personal companion name is never inferred to be an official template', async () => {
  for (const name of ['我的伙伴', 'companion.default']) {
    const hook = await mount(conversation({ companion_session: true, agent_name: name }, name));
    expect(hook.result.current.info?.name).toBe(name);
    await act(async () => { await hook.i18n.changeLanguage('en-US'); });
    expect(hook.result.current.info?.name).toBe(name);
    hook.unmount();
  }
});
