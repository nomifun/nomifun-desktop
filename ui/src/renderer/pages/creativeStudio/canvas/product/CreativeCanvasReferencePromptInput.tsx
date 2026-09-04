/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, {
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

import type { CreativeImagePromptMention } from '../../domain';
import CreativeMediaPreview from '../../assets/components/CreativeMediaPreview';
import styles from './CreativeCanvasReferencePromptInput.module.css';

export interface CreativeCanvasPromptReferenceOption {
  /** Stable canvas node identity. Labels and ordinals are presentation only. */
  nodeId: string;
  label: string;
  kind?: 'image' | 'text';
  textContent?: string;
  mentionLabel?: string;
  thumbnailUrl?: string | null;
  originalUrl?: string | null;
  ordinal: number;
  disabledReason?: string | null;
}

export type CreativeCanvasPromptMentionBinding = CreativeImagePromptMention;

export interface CreativeCanvasReferencePromptChange {
  value: string;
  mentions: CreativeCanvasPromptMentionBinding[];
}

export type CreativeCanvasPromptMentionIssueCode =
  | 'disconnected'
  | 'reference_disabled'
  | 'text_changed';

export interface CreativeCanvasPromptMentionIssue {
  binding: CreativeCanvasPromptMentionBinding;
  code: CreativeCanvasPromptMentionIssueCode;
  reason?: string;
}

export interface CreativeCanvasReferencePromptLabels {
  input: string;
  insertReference: string;
  connectedReferences: string;
  emptyReferences: string;
  disconnectedReference: string;
  changedReferenceText: string;
  unavailableReference: string;
  alreadyMentioned: string;
  results: (count: number) => string;
  referenceOrdinal: (ordinal: number) => string;
  referenceMentionLabel: (ordinal: number) => string;
}

export interface CreativeCanvasReferencePromptInputProps {
  value: string;
  mentions: readonly CreativeCanvasPromptMentionBinding[];
  /** Pass only references directly connected to the active generation node. */
  references: readonly CreativeCanvasPromptReferenceOption[];
  onChange(change: CreativeCanvasReferencePromptChange): void;
  onSubmit?(submission: CreativeCanvasReferencePromptChange): void;
  createMentionId?(reference: CreativeCanvasPromptReferenceOption): string;
  labels?: Partial<CreativeCanvasReferencePromptLabels>;
  id?: string;
  className?: string;
  placeholder?: string;
  disabled?: boolean;
  maxLength?: number;
  autoFocus?: boolean;
  rows?: number;
}

export interface CreativeCanvasPromptMentionTrigger {
  start: number;
  end: number;
  query: string;
}

const DEFAULT_MAX_LENGTH = 1_000_000;
let mentionSequence = 0;

const DEFAULT_LABELS: CreativeCanvasReferencePromptLabels = {
  input: '图片创作提示词',
  insertReference: '引用已连接素材',
  connectedReferences: '已连接参考',
  emptyReferences: '没有匹配的已连接素材',
  disconnectedReference: '引用已断开',
  changedReferenceText: '引用文字已被修改，请重新选择素材',
  unavailableReference: '素材暂不可用',
  alreadyMentioned: '已引用',
  results: (count) => `${count} 个可选素材`,
  referenceOrdinal: (ordinal) => `图 ${ordinal}`,
  referenceMentionLabel: (ordinal) => `图片${ordinal}`,
};

const nextDefaultMentionId = (): string => {
  const randomUuid = globalThis.crypto?.randomUUID;
  if (typeof randomUuid === 'function') return randomUuid.call(globalThis.crypto);
  mentionSequence += 1;
  return `canvas-prompt-mention-${Date.now().toString(36)}-${mentionSequence}`;
};

const mentionToken = (binding: CreativeCanvasPromptMentionBinding): string =>
  `@${binding.fallbackLabel}`;

const isInt = (value: number): boolean => Number.isInteger(value);

const normalizeReferenceLabel = (label: string, ordinal: number): string => {
  const oneLine = label.replace(/[\r\n]+/gu, ' ').trim().replace(/^@+/u, '').trim();
  let bounded = (oneLine || `图 ${ordinal}`).slice(0, 128);
  const trailingCodeUnit = bounded.charCodeAt(bounded.length - 1);
  if (trailingCodeUnit >= 0xd800 && trailingCodeUnit <= 0xdbff) {
    bounded = bounded.slice(0, -1);
  }
  return bounded || `图 ${ordinal}`;
};

const hasValidMentionRange = (
  value: string,
  binding: CreativeCanvasPromptMentionBinding
): boolean =>
  isInt(binding.start) &&
  isInt(binding.end) &&
  binding.start >= 0 &&
  binding.end > binding.start &&
  binding.end <= value.length &&
  value.slice(binding.start, binding.end) === mentionToken(binding);

interface NormalizedCreativeCanvasPromptReference
  extends CreativeCanvasPromptReferenceOption {
  mentionLabel: string;
}

const normalizeReferences = (
  references: readonly CreativeCanvasPromptReferenceOption[],
  referenceMentionLabel: (ordinal: number) => string =
    DEFAULT_LABELS.referenceMentionLabel
): NormalizedCreativeCanvasPromptReference[] => {
  const seen = new Set<string>();
  return references
    .map((reference, index) => ({ reference, index }))
    .sort(
      (left, right) =>
        left.reference.ordinal - right.reference.ordinal || left.index - right.index
    )
    .flatMap(({ reference }) => {
      if (seen.has(reference.nodeId)) return [];
      seen.add(reference.nodeId);
      const label = normalizeReferenceLabel(reference.label, reference.ordinal);
      return [
        {
          ...reference,
          label,
          mentionLabel: reference.disabledReason
            ? label
            : normalizeReferenceLabel(
                reference.mentionLabel ?? referenceMentionLabel(reference.ordinal),
                reference.ordinal
              ),
        },
      ];
    });
};

/**
 * Upgrade durable full-name mentions to compact ordinal aliases without
 * changing their stable source-node identity. Every valid range is rebuilt in
 * one pass so shortening an early token cannot stale later UTF-16 offsets.
 * Malformed external state is returned untouched and remains fail-closed.
 */
export const relabelCreativeCanvasPromptMentions = (
  value: string,
  mentions: readonly CreativeCanvasPromptMentionBinding[],
  references: readonly CreativeCanvasPromptReferenceOption[],
  referenceMentionLabel: (ordinal: number) => string =
    DEFAULT_LABELS.referenceMentionLabel,
  maxLength = DEFAULT_MAX_LENGTH
): CreativeCanvasReferencePromptChange => {
  if (mentions.length === 0) return { value, mentions: [] };

  const sortedMentions = [...mentions].sort(
    (left, right) => left.start - right.start || left.end - right.end
  );
  for (const [index, binding] of sortedMentions.entries()) {
    if (!hasValidMentionRange(value, binding)) {
      return { value, mentions: [...mentions] };
    }
    const previous = sortedMentions[index - 1];
    if (previous && binding.start < previous.end) {
      return { value, mentions: [...mentions] };
    }
  }

  const referencesByNodeId = new Map(
    normalizeReferences(references, referenceMentionLabel)
      .filter((reference) => !reference.disabledReason)
      .map((reference) => [reference.nodeId, reference] as const)
  );
  const nextMentions: CreativeCanvasPromptMentionBinding[] = [];
  let cursor = 0;
  let nextValue = '';
  let changed = false;

  for (const binding of sortedMentions) {
    nextValue += value.slice(cursor, binding.start);
    const reference = referencesByNodeId.get(binding.sourceNodeId);
    const fallbackLabel = reference?.mentionLabel ?? binding.fallbackLabel;
    const token = `@${fallbackLabel}`;
    const start = nextValue.length;
    nextValue += token;
    nextMentions.push({
      ...binding,
      fallbackLabel,
      start,
      end: start + token.length,
    });
    changed ||= fallbackLabel !== binding.fallbackLabel || start !== binding.start;
    cursor = binding.end;
  }
  nextValue += value.slice(cursor);

  if (!changed || nextValue.length > maxLength) {
    return { value, mentions: [...mentions] };
  }
  return { value: nextValue, mentions: nextMentions };
};

export const findCreativeCanvasMentionTrigger = (
  value: string,
  caret: number
): CreativeCanvasPromptMentionTrigger | null => {
  const safeCaret = Math.max(0, Math.min(value.length, caret));
  const prefix = value.slice(0, safeCaret);
  const match = /(?:^|[^A-Za-z0-9._%+\-])@([^\s@]*)$/u.exec(prefix);
  if (!match) return null;
  const start = prefix.lastIndexOf('@');
  return {
    start,
    end: safeCaret,
    query: prefix.slice(start + 1),
  };
};

/**
 * Rebase stable mention occurrences through the single contiguous edit exposed
 * by a textarea change. Any edit overlapping a token unbinds that occurrence;
 * this keeps every emitted draft inside the persistence invariant that a
 * binding's range must exactly match its authored `@label` text.
 */
export const rebaseCreativeCanvasPromptMentions = (
  previousValue: string,
  nextValue: string,
  mentions: readonly CreativeCanvasPromptMentionBinding[]
): CreativeCanvasPromptMentionBinding[] => {
  if (previousValue === nextValue) {
    return mentions.filter((binding) => hasValidMentionRange(nextValue, binding));
  }

  let prefixLength = 0;
  const sharedLength = Math.min(previousValue.length, nextValue.length);
  while (
    prefixLength < sharedLength &&
    previousValue.charCodeAt(prefixLength) === nextValue.charCodeAt(prefixLength)
  ) {
    prefixLength += 1;
  }

  let suffixLength = 0;
  while (
    suffixLength < previousValue.length - prefixLength &&
    suffixLength < nextValue.length - prefixLength &&
    previousValue.charCodeAt(previousValue.length - 1 - suffixLength) ===
      nextValue.charCodeAt(nextValue.length - 1 - suffixLength)
  ) {
    suffixLength += 1;
  }

  const previousEditEnd = previousValue.length - suffixLength;
  const nextEditEnd = nextValue.length - suffixLength;
  const delta = nextEditEnd - previousEditEnd;

  return mentions.flatMap((binding) => {
    let nextBinding = binding;
    if (binding.end <= prefixLength) {
      nextBinding = binding;
    } else if (binding.start >= previousEditEnd) {
      nextBinding = {
        ...binding,
        start: binding.start + delta,
        end: binding.end + delta,
      };
    } else {
      return [];
    }
    return hasValidMentionRange(nextValue, nextBinding) ? [nextBinding] : [];
  });
};

export const collectCreativeCanvasPromptMentionIssues = (
  value: string,
  mentions: readonly CreativeCanvasPromptMentionBinding[],
  references: readonly CreativeCanvasPromptReferenceOption[]
): CreativeCanvasPromptMentionIssue[] => {
  const referencesByNodeId = new Map(
    normalizeReferences(references).map((reference) => [reference.nodeId, reference])
  );
  return mentions.flatMap((binding): CreativeCanvasPromptMentionIssue[] => {
    if (!hasValidMentionRange(value, binding)) {
      return [{ binding, code: 'text_changed' }];
    }
    const reference = referencesByNodeId.get(binding.sourceNodeId);
    if (!reference) return [{ binding, code: 'disconnected' }];
    if (reference.disabledReason) {
      return [
        {
          binding,
          code: 'reference_disabled',
          reason: reference.disabledReason,
        },
      ];
    }
    return [];
  });
};

const isCompositionKeyEvent = (
  event: React.KeyboardEvent<HTMLTextAreaElement>
): boolean =>
  event.nativeEvent.isComposing ||
  // keyCode 229 remains necessary for older embedded Chromium IME events.
  (event.nativeEvent as KeyboardEvent & { keyCode?: number }).keyCode === 229;

const followsTokenWithoutSpace = (suffix: string): boolean =>
  suffix.length === 0 || !/^[\s,.;:!?，。；、！？：)\]}]/u.test(suffix);

const CreativeCanvasReferencePromptInput: React.FC<
  CreativeCanvasReferencePromptInputProps
> = ({
  value,
  mentions,
  references,
  onChange,
  onSubmit,
  createMentionId,
  labels,
  id,
  className,
  placeholder,
  disabled = false,
  maxLength = DEFAULT_MAX_LENGTH,
  autoFocus = false,
  rows = 5,
}) => {
  const generatedId = useId().replaceAll(':', '');
  const inputId = id ?? `creative-canvas-reference-prompt-${generatedId}`;
  const listboxId = `${inputId}-references`;
  const statusId = `${inputId}-reference-status`;
  const rootRef = useRef<HTMLDivElement>(null);
  const textareaRef = useRef<HTMLTextAreaElement>(null);
  const compositionRef = useRef(false);
  const justComposedRef = useRef(false);
  const compositionFrameRef = useRef<number | null>(null);
  const pendingSelectionRef = useRef<number | null>(null);
  const [trigger, setTrigger] =
    useState<CreativeCanvasPromptMentionTrigger | null>(null);
  const [activeNodeId, setActiveNodeId] = useState<string | null>(null);
  const controlLabels = useMemo(
    () => ({ ...DEFAULT_LABELS, ...labels }),
    [labels]
  );
  const normalizedReferences = useMemo(
    () => normalizeReferences(references, controlLabels.referenceMentionLabel),
    [controlLabels.referenceMentionLabel, references]
  );
  const mentionedNodeIds = useMemo(
    () => new Set(mentions.map((binding) => binding.sourceNodeId)),
    [mentions]
  );
  const filteredReferences = useMemo(() => {
    const query = trigger?.query.trim().toLocaleLowerCase() ?? '';
    if (!query) return normalizedReferences;
    return normalizedReferences.filter((reference) =>
      `${reference.mentionLabel} ${reference.label} ${reference.ordinal} ${controlLabels.referenceOrdinal(
        reference.ordinal
      )}`
        .toLocaleLowerCase()
        .includes(query)
    );
  }, [controlLabels, normalizedReferences, trigger?.query]);
  const enabledReferences = useMemo(
    () => filteredReferences.filter((reference) => !reference.disabledReason),
    [filteredReferences]
  );
  const open = trigger !== null && !disabled && !compositionRef.current;
  const issues = useMemo(
    () => collectCreativeCanvasPromptMentionIssues(value, mentions, references),
    [mentions, references, value]
  );
  const activeIndex = filteredReferences.findIndex(
    (reference) => reference.nodeId === activeNodeId && !reference.disabledReason
  );
  const activeOptionId =
    open && activeIndex >= 0 ? `${listboxId}-option-${activeIndex}` : undefined;

  useEffect(() => {
    if (!open) {
      setActiveNodeId(null);
      return;
    }
    if (
      activeNodeId &&
      enabledReferences.some((reference) => reference.nodeId === activeNodeId)
    ) {
      return;
    }
    setActiveNodeId(enabledReferences[0]?.nodeId ?? null);
  }, [activeNodeId, enabledReferences, open]);

  useEffect(() => {
    if (!open) return;
    const dismiss = (event: PointerEvent): void => {
      const target = event.target;
      if (target instanceof Node && !rootRef.current?.contains(target)) {
        setTrigger(null);
      }
    };
    document.addEventListener('pointerdown', dismiss);
    return () => document.removeEventListener('pointerdown', dismiss);
  }, [open]);

  useEffect(
    () => () => {
      if (compositionFrameRef.current !== null) {
        cancelAnimationFrame(compositionFrameRef.current);
      }
    },
    []
  );

  useLayoutEffect(() => {
    const caret = pendingSelectionRef.current;
    if (caret === null) return;
    const textarea = textareaRef.current;
    if (!textarea || textarea.value !== value) return;
    textarea.focus();
    textarea.setSelectionRange(caret, caret);
    pendingSelectionRef.current = null;
  }, [value]);

  const queueSelection = (caret: number, nextValue: string): void => {
    pendingSelectionRef.current = caret;
    queueMicrotask(() => {
      const textarea = textareaRef.current;
      if (!textarea || textarea.value !== nextValue) return;
      textarea.focus();
      textarea.setSelectionRange(caret, caret);
      pendingSelectionRef.current = null;
    });
  };

  const emitChange = (
    nextValue: string,
    nextMentions: CreativeCanvasPromptMentionBinding[]
  ): void => {
    onChange({
      value: nextValue,
      mentions: [...nextMentions].sort(
        (left, right) => left.start - right.start || left.end - right.end
      ),
    });
  };

  const syncTriggerAt = (nextValue: string, caret: number): void => {
    if (compositionRef.current || disabled) {
      setTrigger(null);
      return;
    }
    const nextTrigger = findCreativeCanvasMentionTrigger(nextValue, caret);
    const insideBoundMention =
      nextTrigger !== null &&
      mentions.some(
        (binding) =>
          hasValidMentionRange(nextValue, binding) &&
          nextTrigger.start >= binding.start &&
          nextTrigger.end <= binding.end
      );
    setTrigger(insideBoundMention ? null : nextTrigger);
  };

  const chooseReference = (
    reference: NormalizedCreativeCanvasPromptReference
  ): void => {
    if (!trigger || reference.disabledReason || disabled) return;
    const token = `@${reference.mentionLabel}`;
    const suffix = value.slice(trigger.end);
    const trailingSpace = followsTokenWithoutSpace(suffix) ? ' ' : '';
    const replacement = `${token}${trailingSpace}`;
    const nextValue =
      value.slice(0, trigger.start) + replacement + value.slice(trigger.end);
    if (nextValue.length > maxLength) return;
    const mentionsOutsideReplacement = mentions.filter(
      (binding) =>
        binding.end <= trigger.start || binding.start >= trigger.end
    );
    const nextMentions = rebaseCreativeCanvasPromptMentions(
      value,
      nextValue,
      mentionsOutsideReplacement
    );
    const requestedId = createMentionId?.(reference);
    const proposedId =
      requestedId && requestedId.length <= 128
        ? requestedId
        : nextDefaultMentionId();
    nextMentions.push({
      id: mentions.some((binding) => binding.id === proposedId)
        ? nextDefaultMentionId()
        : proposedId,
      sourceNodeId: reference.nodeId,
      fallbackLabel: reference.mentionLabel,
      start: trigger.start,
      end: trigger.start + token.length,
    });
    const caret = trigger.start + replacement.length;
    emitChange(nextValue, nextMentions);
    setTrigger(null);
    queueSelection(caret, nextValue);
  };

  const openFromTouchButton = (): void => {
    if (disabled) return;
    const textarea = textareaRef.current;
    const selectionStart = textarea?.selectionStart ?? value.length;
    const selectionEnd = textarea?.selectionEnd ?? selectionStart;
    const existingTrigger = findCreativeCanvasMentionTrigger(value, selectionStart);
    const existingTriggerIsBound =
      existingTrigger !== null &&
      mentions.some(
        (binding) =>
          hasValidMentionRange(value, binding) &&
          existingTrigger.start >= binding.start &&
          existingTrigger.end <= binding.end
      );
    if (
      existingTrigger &&
      !existingTriggerIsBound &&
      selectionStart === selectionEnd
    ) {
      setTrigger(existingTrigger);
      textarea?.focus();
      return;
    }
    const prefix = value.slice(0, selectionStart);
    const needsSeparator =
      prefix.length > 0 && !/[\s([{，。；、！？：]$/u.test(prefix);
    const insertion = `${needsSeparator ? ' ' : ''}@`;
    const triggerStart = selectionStart + (needsSeparator ? 1 : 0);
    const nextValue = `${prefix}${insertion}${value.slice(selectionEnd)}`;
    if (nextValue.length > maxLength) return;
    const nextMentions = rebaseCreativeCanvasPromptMentions(
      value,
      nextValue,
      mentions
    );
    const caret = selectionStart + insertion.length;
    emitChange(nextValue, nextMentions);
    setTrigger({ start: triggerStart, end: caret, query: '' });
    queueSelection(caret, nextValue);
  };

  const removeAdjacentMention = (
    key: 'Backspace' | 'Delete',
    selectionStart: number,
    selectionEnd: number
  ): boolean => {
    if (selectionStart !== selectionEnd) return false;
    const binding = mentions.find((candidate) =>
      key === 'Backspace'
        ? candidate.end === selectionStart
        : candidate.start === selectionStart
    );
    if (!binding || !hasValidMentionRange(value, binding)) return false;
    let removalEnd = binding.end;
    if (key === 'Delete') {
      while (removalEnd < value.length && value[removalEnd] === ' ') removalEnd += 1;
    }
    let removalStart = binding.start;
    if (key === 'Backspace' && value[removalEnd] === ' ') removalEnd += 1;
    else if (key === 'Backspace' && value[removalStart - 1] === ' ') {
      removalStart -= 1;
    }
    const nextValue = value.slice(0, removalStart) + value.slice(removalEnd);
    const nextMentions = rebaseCreativeCanvasPromptMentions(
      value,
      nextValue,
      mentions
    );
    emitChange(nextValue, nextMentions);
    setTrigger(null);
    queueSelection(removalStart, nextValue);
    return true;
  };

  const describeIssue = (issue: CreativeCanvasPromptMentionIssue): string => {
    if (issue.reason) return issue.reason;
    if (issue.code === 'disconnected') return controlLabels.disconnectedReference;
    if (issue.code === 'text_changed') return controlLabels.changedReferenceText;
    return controlLabels.unavailableReference;
  };

  const classNames = [styles.root, className].filter(Boolean).join(' ');

  return (
    <div
      ref={rootRef}
      className={classNames}
      data-reference-prompt-input
      data-open={open || undefined}
      data-invalid={issues.length > 0 || undefined}
    >
      <div className={styles.editor}>
        <textarea
          ref={textareaRef}
          id={inputId}
          className={styles.input}
          role='combobox'
          aria-label={controlLabels.input}
          aria-autocomplete='list'
          aria-haspopup='listbox'
          aria-expanded={open}
          aria-controls={open ? listboxId : undefined}
          aria-activedescendant={activeOptionId}
          aria-multiline='true'
          aria-invalid={issues.length > 0 || undefined}
          aria-describedby={issues.length > 0 ? statusId : undefined}
          value={value}
          placeholder={placeholder}
          disabled={disabled}
          maxLength={maxLength}
          autoFocus={autoFocus}
          rows={rows}
          onChange={(event) => {
            let nextValue = event.currentTarget.value;
            let caret = event.currentTarget.selectionStart;
            let nextMentions = rebaseCreativeCanvasPromptMentions(
              value,
              nextValue,
              mentions
            );
            const retainedIds = new Set(nextMentions.map((mention) => mention.id));
            const editedMentions = mentions
              .filter((mention) => !retainedIds.has(mention.id))
              .sort((left, right) => right.start - left.start);
            // A partially edited token becomes ordinary text. Remove its @
            // sigil so it cannot continue looking like a live reference after
            // the stable binding has been intentionally dropped.
            for (const mention of editedMentions) {
              if (nextValue[mention.start] !== '@') continue;
              const before = nextValue;
              nextValue =
                nextValue.slice(0, mention.start) +
                nextValue.slice(mention.start + 1);
              if (mention.start < caret) caret -= 1;
              nextMentions = rebaseCreativeCanvasPromptMentions(
                before,
                nextValue,
                nextMentions
              );
            }
            emitChange(nextValue, nextMentions);
            if (nextValue !== event.currentTarget.value) {
              queueSelection(caret, nextValue);
            }
            if (!compositionRef.current) syncTriggerAt(nextValue, caret);
          }}
          onSelect={(event) => {
            if (!compositionRef.current) {
              syncTriggerAt(
                event.currentTarget.value,
                event.currentTarget.selectionStart
              );
            }
          }}
          onCompositionStartCapture={() => {
            compositionRef.current = true;
            justComposedRef.current = false;
            setTrigger(null);
          }}
          onCompositionEndCapture={(event) => {
            compositionRef.current = false;
            justComposedRef.current = true;
            if (compositionFrameRef.current !== null) {
              cancelAnimationFrame(compositionFrameRef.current);
            }
            compositionFrameRef.current = requestAnimationFrame(() => {
              justComposedRef.current = false;
              compositionFrameRef.current = null;
            });
            syncTriggerAt(event.currentTarget.value, event.currentTarget.selectionStart);
          }}
          onKeyDown={(event) => {
            if (
              compositionRef.current ||
              justComposedRef.current ||
              isCompositionKeyEvent(event)
            ) {
              return;
            }

            if (event.key === 'Backspace' || event.key === 'Delete') {
              if (
                removeAdjacentMention(
                  event.key,
                  event.currentTarget.selectionStart,
                  event.currentTarget.selectionEnd
                )
              ) {
                event.preventDefault();
                return;
              }
            }

            if (open && event.key === 'Escape') {
              event.preventDefault();
              event.stopPropagation();
              setTrigger(null);
              return;
            }

            if (open && event.key === 'Tab') {
              setTrigger(null);
              return;
            }

            if (open && (event.key === 'ArrowDown' || event.key === 'ArrowUp')) {
              event.preventDefault();
              if (enabledReferences.length === 0) return;
              const currentIndex = enabledReferences.findIndex(
                (reference) => reference.nodeId === activeNodeId
              );
              const direction = event.key === 'ArrowDown' ? 1 : -1;
              const nextIndex =
                currentIndex < 0
                  ? direction > 0
                    ? 0
                    : enabledReferences.length - 1
                  : (currentIndex + direction + enabledReferences.length) %
                    enabledReferences.length;
              setActiveNodeId(enabledReferences[nextIndex]?.nodeId ?? null);
              return;
            }

            if (event.key !== 'Enter' || event.shiftKey) return;
            if (open) {
              event.preventDefault();
              const selected =
                enabledReferences.find(
                  (reference) => reference.nodeId === activeNodeId
                ) ?? enabledReferences[0];
              if (selected) chooseReference(selected);
              return;
            }
            if (onSubmit) {
              event.preventDefault();
              if ((value.trim().length > 0 || references.some((reference) =>
                reference.kind === 'text' && reference.textContent?.trim() && !reference.disabledReason
              )) && issues.length === 0) {
                onSubmit({ value, mentions: [...mentions] });
              }
            }
          }}
        />

        <button
          type='button'
          className={styles.mentionButton}
          aria-label={controlLabels.insertReference}
          title={controlLabels.insertReference}
          disabled={disabled}
          onMouseDown={(event) => event.preventDefault()}
          onClick={openFromTouchButton}
        >
          @
        </button>

        {open ? (
          <div className={styles.popup}>
            <div className={styles.popupHeader}>
              <span>{controlLabels.connectedReferences}</span>
              <span className={styles.resultCount} aria-live='polite'>
                {controlLabels.results(enabledReferences.length)}
              </span>
            </div>
            <div
              id={listboxId}
              className={styles.listbox}
              role='listbox'
              aria-label={controlLabels.connectedReferences}
            >
              {filteredReferences.length > 0 ? (
                filteredReferences.map((reference, index) => {
                  const optionId = `${listboxId}-option-${index}`;
                  const unavailable = Boolean(reference.disabledReason);
                  const active = reference.nodeId === activeNodeId && !unavailable;
                  return (
                    <button
                      key={reference.nodeId}
                      id={optionId}
                      type='button'
                      role='option'
                      className={styles.option}
                      aria-selected={active}
                      aria-disabled={unavailable || undefined}
                      data-active={active || undefined}
                      data-unavailable={unavailable || undefined}
                      disabled={unavailable}
                      onMouseDown={(event) => event.preventDefault()}
                      onMouseEnter={() => {
                        if (!unavailable) setActiveNodeId(reference.nodeId);
                      }}
                      onClick={() => chooseReference(reference)}
                    >
                      <span className={styles.thumbnail} aria-hidden='true'>
                        {reference.kind === 'text' ? (
                          <span className={styles.thumbnailText}>{reference.textContent}</span>
                        ) : reference.thumbnailUrl || reference.originalUrl ? (
                          <CreativeMediaPreview
                            kind='image'
                            src={reference.originalUrl ?? reference.thumbnailUrl}
                            posterSrc={reference.thumbnailUrl}
                            alt=''
                          />
                        ) : (
                          <span className={styles.thumbnailFallback}>@</span>
                        )}
                      </span>
                      <span className={styles.optionContent}>
                        <span className={styles.optionTitle}>
                          @{reference.mentionLabel}
                        </span>
                        <span className={styles.optionMeta}>
                          {reference.disabledReason
                            ? reference.disabledReason
                            : mentionedNodeIds.has(reference.nodeId)
                              ? `${reference.label} · ${reference.kind === 'text' ? reference.mentionLabel : controlLabels.referenceOrdinal(reference.ordinal)} · ${controlLabels.alreadyMentioned}`
                              : `${reference.label} · ${reference.kind === 'text' ? reference.mentionLabel : controlLabels.referenceOrdinal(reference.ordinal)}`}
                        </span>
                      </span>
                    </button>
                  );
                })
              ) : (
                <div className={styles.empty} role='status'>
                  {controlLabels.emptyReferences}
                </div>
              )}
            </div>
          </div>
        ) : null}
      </div>

      {issues.length > 0 ? (
        <div id={statusId} className={styles.issues} aria-live='polite'>
          {issues.map((issue, index) => (
            <span
              key={`${issue.binding.id}:${issue.binding.start}:${index}`}
              className={styles.issue}
            >
              <span className={styles.issueToken}>
                @{issue.binding.fallbackLabel}
              </span>
              <span aria-hidden='true'> · </span>
              {describeIssue(issue)}
            </span>
          ))}
        </div>
      ) : null}
    </div>
  );
};

export default CreativeCanvasReferencePromptInput;
