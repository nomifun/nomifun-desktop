/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import '../../../../../../test/setup-dom.ts';

import { cleanup, fireEvent, render, within } from '@testing-library/react';
import { afterEach, describe, expect, test } from 'bun:test';
import React, { useState } from 'react';

import CreativeCanvasReferencePromptInput, {
  collectCreativeCanvasPromptMentionIssues,
  findCreativeCanvasMentionTrigger,
  rebaseCreativeCanvasPromptMentions,
  relabelCreativeCanvasPromptMentions,
  type CreativeCanvasPromptMentionBinding,
  type CreativeCanvasPromptReferenceOption,
  type CreativeCanvasReferencePromptChange,
} from './CreativeCanvasReferencePromptInput';

afterEach(() => cleanup());

const references: readonly CreativeCanvasPromptReferenceOption[] = [
  {
    nodeId: 'person-node',
    label: '人物图',
    thumbnailUrl: 'https://example.test/person.png',
    ordinal: 1,
  },
  {
    nodeId: 'clothes-node',
    label: '服装图',
    thumbnailUrl: 'https://example.test/clothes.png',
    ordinal: 2,
  },
];

interface HarnessProps {
  initial?: CreativeCanvasReferencePromptChange;
  referenceOptions?: readonly CreativeCanvasPromptReferenceOption[];
  onState?(state: CreativeCanvasReferencePromptChange): void;
  onSubmit?(state: CreativeCanvasReferencePromptChange): void;
}

const Harness: React.FC<HarnessProps> = ({
  initial = { value: '', mentions: [] },
  referenceOptions = references,
  onState,
  onSubmit,
}) => {
  const [state, setState] = useState(initial);
  return (
    <CreativeCanvasReferencePromptInput
      value={state.value}
      mentions={state.mentions}
      references={referenceOptions}
      createMentionId={(reference) => `mention-${reference.nodeId}`}
      placeholder='描述要生成的图片内容'
      onChange={(next) => {
        setState(next);
        onState?.(next);
      }}
      onSubmit={onSubmit}
    />
  );
};

const typeAtCaret = (
  textarea: HTMLTextAreaElement,
  value: string,
  caret = value.length
): void => {
  fireEvent.change(textarea, { target: { value } });
  textarea.setSelectionRange(caret, caret);
  fireEvent.select(textarea);
};

describe('CreativeCanvasReferencePromptInput', () => {
  test('keeps a reference selectable when its thumbnail falls back to the original', () => {
    const reference = {
      ...references[0]!,
      originalUrl: 'https://example.test/person-original.png',
    };
    const { getByRole } = render(<Harness referenceOptions={[reference]} />);
    const input = getByRole('combobox') as HTMLTextAreaElement;
    typeAtCaret(input, '@');
    const option = getByRole('option', { name: /@图片1.*人物图/ });
    const image = option.querySelector('img')!;
    expect(image.getAttribute('src')).toBe(reference.thumbnailUrl!);

    fireEvent.error(image);
    expect(option.querySelector('img')?.getAttribute('src')).toBe(reference.originalUrl);
    fireEvent.click(option);
    expect(input.value).toBe('@图片1 ');
  });

  test('opens a connected-reference-only list after @ and exposes combobox semantics', () => {
    const { getByRole } = render(<Harness />);
    const input = getByRole('combobox', {
      name: '图片创作提示词',
    }) as HTMLTextAreaElement;

    typeAtCaret(input, '@');

    expect(input.getAttribute('aria-expanded')).toBe('true');
    expect(input.getAttribute('aria-controls')).not.toBeNull();
    const listbox = getByRole('listbox');
    expect(within(listbox).getAllByRole('option').length).toBe(2);
    expect(within(listbox).getByRole('option', { name: /@图片1.*人物图/ })).toBeDefined();
    expect(within(listbox).getByRole('option', { name: /@图片2.*服装图/ })).toBeDefined();
  });

  test('opens after adjacent Chinese text but not inside an existing bound token', () => {
    const mention: CreativeCanvasPromptMentionBinding = {
      id: 'mention-person',
      sourceNodeId: 'person-node',
      fallbackLabel: '人物图',
      start: 1,
      end: 5,
    };
    const { getByRole, queryByRole } = render(
      <Harness initial={{ value: '让@人物图', mentions: [mention] }} />
    );
    const input = getByRole('combobox') as HTMLTextAreaElement;

    input.setSelectionRange(5, 5);
    fireEvent.select(input);
    expect(queryByRole('listbox')).toBeNull();

    typeAtCaret(input, '让@');
    expect(getByRole('listbox')).toBeDefined();
  });

  test('inserts a plain @label while returning a stable node binding', () => {
    let latest: CreativeCanvasReferencePromptChange | undefined;
    const { getByRole } = render(<Harness onState={(state) => (latest = state)} />);
    const input = getByRole('combobox') as HTMLTextAreaElement;

    typeAtCaret(input, '让 @', 3);
    fireEvent.click(getByRole('option', { name: /@图片1/ }));

    expect(input.value).toBe('让 @图片1 ');
    expect(latest?.mentions).toEqual([
      {
        id: 'mention-person-node',
        sourceNodeId: 'person-node',
        fallbackLabel: '图片1',
        start: 2,
        end: 6,
      },
    ]);
  });

  test('keeps the asset name searchable while persisting only its ordinal alias', () => {
    let latest: CreativeCanvasReferencePromptChange | undefined;
    const { getByRole } = render(
      <Harness
        referenceOptions={[
          {
            nodeId: 'person-node',
            label: '  @人物\n参考  ',
            ordinal: 1,
          },
        ]}
        onState={(state) => (latest = state)}
      />
    );
    const input = getByRole('combobox') as HTMLTextAreaElement;

    typeAtCaret(input, '@');
    fireEvent.click(getByRole('option', { name: /@图片1.*人物 参考/ }));

    expect(input.value).toBe('@图片1 ');
    expect(latest?.mentions[0]?.fallbackLabel).toBe('图片1');
  });

  test('filters, navigates with arrows, selects with Enter, and dismisses with Escape', () => {
    const { getByRole, queryByRole } = render(<Harness />);
    const input = getByRole('combobox') as HTMLTextAreaElement;

    typeAtCaret(input, '@服');
    expect(getByRole('listbox').querySelectorAll('[role="option"]').length).toBe(1);
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(input.value).toBe('@图片2 ');
    expect(queryByRole('listbox')).toBeNull();

    typeAtCaret(input, `${input.value}@`);
    fireEvent.keyDown(input, { key: 'ArrowDown' });
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(input.value.endsWith('@图片2 ')).toBe(true);

    typeAtCaret(input, `${input.value}@`);
    expect(getByRole('listbox')).toBeDefined();
    fireEvent.keyDown(input, { key: 'Escape' });
    expect(queryByRole('listbox')).toBeNull();
    expect(input.getAttribute('aria-expanded')).toBe('false');
  });

  test('uses the touch @ button and keeps disabled references visible but unselectable', () => {
    const referenceOptions: readonly CreativeCanvasPromptReferenceOption[] = [
      references[0]!,
      { ...references[1]!, disabledReason: '图片上传中' },
    ];
    const { getByRole } = render(
      <Harness initial={{ value: '生成', mentions: [] }} referenceOptions={referenceOptions} />
    );
    const input = getByRole('combobox') as HTMLTextAreaElement;
    input.setSelectionRange(2, 2);

    fireEvent.click(getByRole('button', { name: '引用已连接素材' }));

    expect(input.value).toBe('生成 @');
    const unavailable = getByRole('option', { name: /@服装图.*图片上传中/ });
    expect(unavailable.getAttribute('aria-disabled')).toBe('true');
    expect((unavailable as HTMLButtonElement).disabled).toBe(true);
  });

  test('does not choose or submit for native IME key signals', () => {
    const submissions: CreativeCanvasReferencePromptChange[] = [];
    const { getByRole, queryByRole } = render(
      <Harness
        initial={{ value: '换衣', mentions: [] }}
        onSubmit={(state) => submissions.push(state)}
      />
    );
    const input = getByRole('combobox') as HTMLTextAreaElement;

    fireEvent.keyDown(input, { key: 'Enter', isComposing: true, keyCode: 229 });
    expect(submissions).toEqual([]);
    expect(queryByRole('listbox')).toBeNull();
  });

  test('submits with Enter, leaves Shift+Enter native, and atomically deletes a mention', () => {
    const mention: CreativeCanvasPromptMentionBinding = {
      id: 'mention-person',
      sourceNodeId: 'person-node',
      fallbackLabel: '人物图',
      start: 0,
      end: 4,
    };
    const submissions: CreativeCanvasReferencePromptChange[] = [];
    const { getByRole } = render(
      <Harness
        initial={{ value: '@人物图 保持五官', mentions: [mention] }}
        onSubmit={(state) => submissions.push(state)}
      />
    );
    const input = getByRole('combobox') as HTMLTextAreaElement;

    expect(fireEvent.keyDown(input, { key: 'Enter', shiftKey: true })).toBe(true);
    expect(submissions).toEqual([]);
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(submissions).toEqual([
      { value: '@人物图 保持五官', mentions: [mention] },
    ]);

    input.setSelectionRange(4, 4);
    fireEvent.keyDown(input, { key: 'Backspace' });
    expect(input.value).toBe('保持五官');
  });

  test('removes the @ sigil when a bound token is partially edited', () => {
    const mention: CreativeCanvasPromptMentionBinding = {
      id: 'mention-person',
      sourceNodeId: 'person-node',
      fallbackLabel: '人物图',
      start: 0,
      end: 4,
    };
    let latest: CreativeCanvasReferencePromptChange | undefined;
    const { getByRole } = render(
      <Harness
        initial={{ value: '@人物图 出镜', mentions: [mention] }}
        onState={(state) => (latest = state)}
      />
    );
    const input = getByRole('combobox') as HTMLTextAreaElement;

    typeAtCaret(input, '@人物 出镜', 3);

    expect(latest).toEqual({ value: '人物 出镜', mentions: [] });
  });

  test('shows disconnected bindings as invalid and blocks Enter submission', () => {
    const submissions: CreativeCanvasReferencePromptChange[] = [];
    const disconnected: CreativeCanvasPromptMentionBinding = {
      id: 'mention-clothes',
      sourceNodeId: 'clothes-node',
      fallbackLabel: '服装图',
      start: 0,
      end: 4,
    };
    const { getByRole } = render(
      <Harness
        initial={{ value: '@服装图 是服装', mentions: [disconnected] }}
        referenceOptions={[references[0]!]}
        onSubmit={(state) => submissions.push(state)}
      />
    );
    const input = getByRole('combobox') as HTMLTextAreaElement;

    expect(input.getAttribute('aria-invalid')).toBe('true');
    const status = document.getElementById(
      input.getAttribute('aria-describedby') ?? ''
    );
    expect(status?.textContent?.includes('@服装图')).toBe(true);
    expect(status?.textContent?.includes('引用已断开')).toBe(true);
    fireEvent.keyDown(input, { key: 'Enter' });
    expect(submissions).toEqual([]);
  });

  test('rebases valid mentions and unbinds any edited token', () => {
    const mention: CreativeCanvasPromptMentionBinding = {
      id: 'mention-person',
      sourceNodeId: 'person-node',
      fallbackLabel: '人物图',
      start: 2,
      end: 6,
    };
    expect(
      rebaseCreativeCanvasPromptMentions('让 @人物图 出镜', '请让 @人物图 出镜', [mention])
    ).toEqual([{ ...mention, start: 3, end: 7 }]);
    expect(
      rebaseCreativeCanvasPromptMentions('让 @人物图 出镜', '让 @人物 出镜', [mention])
    ).toEqual([]);
    expect(
      rebaseCreativeCanvasPromptMentions('让 @人物图 出镜', '让  出镜', [mention])
    ).toEqual([]);
  });

  test('atomically relabels legacy mentions and rebases later disconnected ranges', () => {
    const value = '让 @很长的人物参考图 穿 @服装图，并保留 @失效参考';
    const personStart = value.indexOf('@很长的人物参考图');
    const clothesStart = value.indexOf('@服装图');
    const disconnectedStart = value.indexOf('@失效参考');
    const legacyMentions: CreativeCanvasPromptMentionBinding[] = [
      {
        id: 'mention-person',
        sourceNodeId: 'person-node',
        fallbackLabel: '很长的人物参考图',
        start: personStart,
        end: personStart + '@很长的人物参考图'.length,
      },
      {
        id: 'mention-clothes',
        sourceNodeId: 'clothes-node',
        fallbackLabel: '服装图',
        start: clothesStart,
        end: clothesStart + '@服装图'.length,
      },
      {
        id: 'mention-disconnected',
        sourceNodeId: 'disconnected-node',
        fallbackLabel: '失效参考',
        start: disconnectedStart,
        end: disconnectedStart + '@失效参考'.length,
      },
    ];

    const relabeled = relabelCreativeCanvasPromptMentions(
      value,
      legacyMentions,
      references
    );

    expect(relabeled.value).toBe('让 @图片1 穿 @图片2，并保留 @失效参考');
    expect(
      relabeled.mentions.map((mention) => ({
        sourceNodeId: mention.sourceNodeId,
        fallbackLabel: mention.fallbackLabel,
        token: relabeled.value.slice(mention.start, mention.end),
      }))
    ).toEqual([
      {
        sourceNodeId: 'person-node',
        fallbackLabel: '图片1',
        token: '@图片1',
      },
      {
        sourceNodeId: 'clothes-node',
        fallbackLabel: '图片2',
        token: '@图片2',
      },
      {
        sourceNodeId: 'disconnected-node',
        fallbackLabel: '失效参考',
        token: '@失效参考',
      },
    ]);

    const malformed = [{ ...legacyMentions[0]!, end: 2 }];
    expect(
      relabelCreativeCanvasPromptMentions(value, malformed, references)
    ).toEqual({ value, mentions: malformed });
  });

  test('finds only an active @ query and reports disabled-reference issues', () => {
    expect(findCreativeCanvasMentionTrigger('让 @人物', 5)).toEqual({
      start: 2,
      end: 5,
      query: '人物',
    });
    expect(findCreativeCanvasMentionTrigger('让@人物', 4)).toEqual({
      start: 1,
      end: 4,
      query: '人物',
    });
    expect(findCreativeCanvasMentionTrigger('mail@example.com', 16)).toBeNull();
    expect(findCreativeCanvasMentionTrigger('@人物 是人物', 7)).toBeNull();

    const binding: CreativeCanvasPromptMentionBinding = {
      id: 'mention-person',
      sourceNodeId: 'person-node',
      fallbackLabel: '人物图',
      start: 0,
      end: 4,
    };
    expect(
      collectCreativeCanvasPromptMentionIssues('@人物图', [binding], [
        { ...references[0]!, disabledReason: '素材处理中' },
      ])
    ).toEqual([
      {
        binding,
        code: 'reference_disabled',
        reason: '素材处理中',
      },
    ]);
  });
});
