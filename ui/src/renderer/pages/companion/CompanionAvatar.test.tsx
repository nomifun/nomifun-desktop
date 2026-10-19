import { describe, expect, test } from 'bun:test';
import { renderToStaticMarkup } from 'react-dom/server';
import { parseCompanionId, parseFigureId } from '@/common/types/ids';
import enNomi from '@renderer/services/i18n/locales/en-US/nomi.json';
import zhNomi from '@renderer/services/i18n/locales/zh-CN/nomi.json';
import CompanionAvatar from './CompanionAvatar';
import { CHARACTERS, DEFAULT_CHARACTER_ID, getCharacter } from './characters';

describe('official companion characters', () => {
  test('offers Mochi and Bolt with renderable names and components', () => {
    expect(CHARACTERS.map(({ id }) => id)).toEqual(['mochi', 'bolt']);
    expect(enNomi.characters.mochi.name).toBe('Mochi');
    expect(zhNomi.characters.mochi.name).toBe('团子 Mochi');
    expect(enNomi.characters.bolt.name).toBe('Bolt');
    expect(zhNomi.characters.bolt.name).toBe('波特 Bolt');
    for (const { id } of CHARACTERS) {
      const html = renderToStaticMarkup(<CompanionAvatar character={id} mood='happy' activity='thinking' size={88} />);
      expect(html).toContain(`nomi-${id}--happy`);
      expect(html).toContain(`nomi-${id}--thinking`);
      expect(html).toContain('width:88px;height:88px');
    }
  });

  test.each(['ink', 'no-such-character', '', null, undefined])('renders an unavailable character (%s) as Mochi', (character) => {
    expect(getCharacter(character).id).toBe(DEFAULT_CHARACTER_ID);
    const avatar = (id?: string | null) => renderToStaticMarkup(
      <CompanionAvatar character={id} mood='sleepy' activity='idle' size={120} />
    );
    expect(avatar(character)).toBe(avatar('mochi'));
  });

  test('continues rendering a custom library figure through its own image URL', () => {
    const figureId = parseFigureId('019f0000-0000-7000-8000-000000000001');
    const html = renderToStaticMarkup(<CompanionAvatar
      character='custom'
      companionId={parseCompanionId('019f0000-0000-7000-8000-000000000002')}
      customFigure={{ figureId, aspect: 0.8, headBox: { x: 0.2, y: 0.1, w: 0.6, h: 0.4 }, sizeTier: 'm' }}
      mood='excited'
      activity='thinking'
      size={150}
    />);
    expect(html).toContain('nomi-cfig--excited');
    expect(html).toContain('nomi-cfig--thinking');
    expect(html).toContain(`/api/companion/figures/${figureId}/image?`);
    expect(html).not.toContain('nomi-mochi');
  });
});
