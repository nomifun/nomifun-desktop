import { afterEach, describe, expect, mock, spyOn, test } from 'bun:test';
import { cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { Message } from '@arco-design/web-react';
import { parseCompanionId } from '@/common/types/ids';
import { recordInstalledMarketItem, writeInstalledMarketState } from '@/renderer/services/skills/skillMarketProvenance';
import { AsideHost } from '../../AsideHost';
import type { WorkspaceTabProps } from '../../types';
import SkillsTab from './index';
import * as dataHook from './useSkillsTabData';

const locale = createInstance();
await locale.init({ lng: 'en-US', resources: { 'en-US': { translation: {} } } });
afterEach(() => { cleanup(); writeInstalledMarketState({}); });

describe('companion catalog skill actions', () => {
  test.each(['row', 'detail'])('revokes the canonical name, not the market/localized title (%s)', async (surface) => {
    const canonicalName = 'dev-expert';
    const displayName = '编程专家.Skill';
    const skill = { name: canonicalName, description: 'Canonical summary', source: 'custom', location: 'C:/skills/dev-expert/SKILL.md' };
    writeInstalledMarketState(recordInstalledMarketItem({}, {
      id: 'skillhub:owner/dev-expert', source: 'skillhub', rank: 1,
      name: displayName, description: 'Official market summary',
      url: 'https://skillhub.cn/skills/owner/dev-expert', install_command: 'npx skills add @owner/dev-expert',
    }, [canonicalName]));
    const data = spyOn(dataHook, 'useSkillsTabData').mockReturnValue({
      catalog: [skill], autoNames: new Set(), generated: [], loading: false, initialLoading: false,
      refresh: async () => {}, decide: async () => true, learnFromSession: async () => true,
    });
    const notification = spyOn(Message, 'success').mockImplementation(() => (() => {}) as ReturnType<typeof Message.success>);
    const patchCompanion = mock(async () => undefined);
    const companion = { profile: { skills: { enabled: [canonicalName], disabled_auto: [] } }, loading: false, patchCompanion } as unknown as WorkspaceTabProps['companion'];
    try {
      const view = render(<I18nextProvider i18n={locale}><AsideHost><SkillsTab companionId={parseCompanionId('019b0000-0000-7000-8000-000000000001')} companion={companion} /></AsideHost></I18nextProvider>);
      expect(view.getByText(displayName)).toBeTruthy();
      expect(view.getByText('Official market summary')).toBeTruthy();
      if (surface === 'row') fireEvent.click(view.getByRole('switch', { name: '取消授予' }));
      else {
        fireEvent.click(view.getByText(displayName));
        fireEvent.click(await view.findByRole('button', { name: '取消授予' }));
      }
      await waitFor(() => expect(patchCompanion).toHaveBeenCalledWith({ skills: { enabled: [], disabled_auto: [] } }));
    } finally { data.mockRestore(); notification.mockRestore(); }
  });
});
