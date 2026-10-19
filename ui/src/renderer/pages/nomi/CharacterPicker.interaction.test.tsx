import { afterEach, beforeEach, expect, mock, spyOn, test } from 'bun:test';
import { cleanup, fireEvent, render } from '@testing-library/react';
import { createInstance } from 'i18next';
import { I18nextProvider } from 'react-i18next';
import type { IFigureMeta } from '@/common/adapter/ipcBridge';
import { parseFigureId } from '@/common/types/ids';
import nomi from '@renderer/services/i18n/locales/en-US/nomi.json';
import * as figureStore from './useFigures';
import CharacterPicker from './CharacterPicker';

const i18n = createInstance();
await i18n.init({ lng: 'en-US', resources: { 'en-US': { translation: { nomi } } } });

const figure: IFigureMeta = {
  figure_id: parseFigureId('019f0000-0000-7000-8000-000000000001'),
  name: 'My custom figure', aspect: 0.8,
  head_box: { x: 0.2, y: 0.1, w: 0.6, h: 0.4 }, size_tier: 'm', created_at: 1,
};

beforeEach(() => {
  spyOn(figureStore, 'useFigures').mockReturnValue({
    figures: [figure], loading: false, loaded: true,
    refresh: async () => {}, remove: async () => {}, add: () => {},
    rename: async () => figure, update: async () => figure,
  });
  spyOn(figureStore, 'useFiguresInUse').mockReturnValue(new Set());
});
afterEach(() => { cleanup(); mock.restore(); });

test.each([false, true])('creation and compact selection keep the two official looks and custom figures (compact=%s)', (compact) => {
  const selectCharacter = mock(() => {});
  const selectFigure = mock(() => {});
  const view = render(<I18nextProvider i18n={i18n}>
    <CharacterPicker value='ink' compact={compact} onSelectCharacter={selectCharacter} onSelectFigure={selectFigure} />
  </I18nextProvider>);

  expect(view.container.querySelectorAll('.nomi-ch')).toHaveLength(2);
  expect(view.queryByText('Ink')).toBeNull();
  expect(selectCharacter).not.toHaveBeenCalled();
  fireEvent.click(view.getByText(nomi.characters.mochi.name));
  expect(selectCharacter).toHaveBeenLastCalledWith('mochi');
  fireEvent.click(view.getByText(nomi.characters.bolt.name));
  expect(selectCharacter).toHaveBeenLastCalledWith('bolt');
  fireEvent.click(view.getByText(figure.name));
  expect(selectFigure).toHaveBeenLastCalledWith(figure);

  fireEvent.click(view.getByText(nomi.customFigure.createNew));
  expect(view.getByText(nomi.customFigure.wizardTitle)).toBeTruthy();
  expect(document.querySelector('input[type="file"]')).toBeTruthy();
});
