import '../../../../test/setup-dom.ts';
import '@arco-design/web-react/lib/_util/react-19-adapter';
import { act, cleanup, fireEvent, render, waitFor } from '@testing-library/react';
import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import { MemoryRouter } from 'react-router-dom';
import { SWRConfig } from 'swr';
import { configService } from '@/common/config/configService';
import { ipcBridge } from '@/common';
import { browserStorageKey, setBrowserStorageGeneration } from '@/common/utils/browserStorageKey';
import { uuidv7 } from '@/common/utils';
import { parseTerminalId } from '@/common/types/ids';
import * as themeContext from '@/renderer/hooks/context/ThemeContext';
import { PreviewProvider } from '@/renderer/pages/conversation/Preview';
import { MessageListProvider } from '@/renderer/pages/conversation/Messages/hooks';
import messages from '@/renderer/services/i18n/locales/en-US/messages.json';
import terminal from '@/renderer/services/i18n/locales/en-US/terminal.json';
import common from '@/renderer/services/i18n/locales/en-US/common.json';
import TerminalSendBox from './TerminalSendBox';
import type { XtermViewHandle } from './XtermView';

const i18n = createInstance();
await i18n.init({ lng: 'en-US', resources: { 'en-US': { translation: { messages, terminal, common } } } });
beforeEach(() => {
  configService.reset(); setBrowserStorageGeneration(uuidv7());
  spyOn(themeContext, 'useThemeContext').mockReturnValue({ theme: 'light', colorScheme: 'default', fontScale: 1,
    setTheme: async () => {}, setColorScheme: async () => {}, setFontScale: async () => {} });
  spyOn(ipcBridge.mode.listProviders, 'invoke').mockResolvedValue([]);
  for (const event of [ipcBridge.mode.onProvidersChanged, ipcBridge.conversation.reconnected, ipcBridge.conversation.turnStarted,
    ipcBridge.conversation.turnCompleted, ipcBridge.agentPlatform.sessions.onAgentChanged, ipcBridge.agentPlatform.sessions.onCapabilitiesChanged,
    ipcBridge.conversation.responseStream, ipcBridge.knowledge.onTreeChanged, ipcBridge.knowledge.onEntryContentUpdated,
    ipcBridge.fileStream.contentUpdate]) spyOn(event, 'on').mockImplementation(() => () => {});
});
afterEach(() => { cleanup(); mock.restore(); configService.reset(); });

function fixture() {
  const terminalId = parseTerminalId(uuidv7());
  const write = mock(async (_text: string) => {});
  const clear = mock();
  const api: { current: XtermViewHandle } = { current: { writeToPty: write, isBracketedPaste: () => false,
    clear: mock(), reset: mock(), focus: mock() } };
  const page = render(<MemoryRouter><I18nextProvider i18n={i18n}><SWRConfig value={{ provider: () => new Map(), shouldRetryOnError: false }}>
    <PreviewProvider persistNamespace={browserStorageKey('terminal-command-test', 'terminal', terminalId)} subscribeGlobalOpen={false}><MessageListProvider>
      <TerminalSendBox commandTargetKey='terminal:fixture' terminalApi={api} onClearView={clear} />
    </MessageListProvider></PreviewProvider>
  </SWRConfig></I18nextProvider></MemoryRouter>);
  const input = page.getByTestId('sendbox-input') as HTMLTextAreaElement;
  return { page, input, write, clear, change: (value: string) => fireEvent.change(input, { target: { value } }) };
}

test.each(['button', 'keyboard', 'menu'])('real terminal composer clears its view for %s without writing command bytes', async gesture => {
  const f = fixture();
  f.change('/');
  await waitFor(() => expect(f.page.getAllByRole('option')).toHaveLength(2));
  expect(f.page.queryByRole('option', { name: /\/open/ })).toBeNull();
  expect(f.page.queryByRole('option', { name: /\/copy/ })).toBeNull();
  f.change('/clear');
  if (gesture === 'button') fireEvent.click(f.page.getByTestId('sendbox-send-btn'));
  else if (gesture === 'keyboard') fireEvent.keyDown(f.input, { key: 'Enter' });
  else fireEvent.click(await f.page.findByRole('option', { name: /\/clear/ }));
  await waitFor(() => expect(f.clear).toHaveBeenCalledTimes(1));
  expect(f.input.value).toBe(''); expect(f.write).not.toHaveBeenCalled();
});

test('real terminal interrupt sends ETX while unknown absolute paths retain the ordinary PTY byte contract', async () => {
  const f = fixture();
  f.change('/interrupt'); fireEvent.click(f.page.getByTestId('sendbox-send-btn'));
  await waitFor(() => expect(f.write).toHaveBeenCalledWith('\x03'));
  await waitFor(() => expect(f.input.value).toBe(''));
  f.change('/usr/bin/env'); fireEvent.click(f.page.getByTestId('sendbox-send-btn'));
  await waitFor(() => expect(f.write).toHaveBeenCalledWith('/usr/bin/env\r'));
  await act(async () => {});
  expect(f.clear).not.toHaveBeenCalled();
});
