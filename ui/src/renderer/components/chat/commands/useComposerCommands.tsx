import { Message } from '@arco-design/web-react';
import { useCallback, useEffect, useMemo, useRef, useState, type KeyboardEvent, type RefObject } from 'react';
import { useTranslation } from 'react-i18next';
import { parseError, uuidv7 } from '@/common/utils';
import { isImeComposingKey, isSystemTextEditingShortcut } from '@/renderer/hooks/chat/useCompositionInput';
import SlashCommandMenu from '../SlashCommandMenu';
import { ComposerCommandOverlay } from './ComposerCommandOverlay';
import { commandQuery, resolveComposerCommand, validateComposerCommands, type ComposerCommand, type ComposerCommandPanel } from './types';

export type { ComposerCommand, ComposerCommandPanel } from './types';

/** Shared interaction, never a Session owner or a second delivery queue. Every
 * host calls dispatchInput before its ordinary submit/edit/steer path. */
export function useComposerCommands({ targetKey, input, setInput, commands, anchorRef, panel, onError }: {
  targetKey: string;
  input: string;
  setInput: (value: string) => void;
  commands: readonly ComposerCommand[];
  anchorRef: RefObject<HTMLElement | null>;
  panel?: ComposerCommandPanel;
  onError?: (error: unknown) => void;
}) {
  const { t } = useTranslation();
  const [activeIndex, setActiveIndex] = useState(0);
  const [dismissed, setDismissed] = useState(false);
  const [busyTarget, setBusyTarget] = useState<string>();
  const active = useRef(true);
  const latest = useRef({ targetKey, input, version: 0, generation: 0, setInput, onError });
  if (latest.current.targetKey !== targetKey || latest.current.input !== input) {
    latest.current.version++;
    if (latest.current.targetKey !== targetKey) latest.current.generation++;
  }
  Object.assign(latest.current, { targetKey, input, setInput, onError });
  const pending = useRef<{ targetKey: string; generation: number } | undefined>(undefined);
  // A retry of an unchanged command is the same UI action. In particular, a
  // lost clear response must not mint a new clear that can erase later work.
  const attempt = useRef<{ targetKey: string; generation: number; version: number; commandId: string; key: string } | undefined>(undefined);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  const query = commandQuery(input);
  useMemo(() => validateComposerCommands(commands), [commands]);
  useEffect(() => { setActiveIndex(0); setDismissed(false); }, [query, targetKey]);
  const filtered = useMemo(() => query === null ? [] : commands.filter(command =>
    command.name.toLowerCase().startsWith(query.toLowerCase())), [commands, query]);
  const safeIndex = Math.min(activeIndex, Math.max(0, filtered.length - 1));
  const menuOpen = query !== null && !dismissed && filtered.length > 0 && !panel?.isOpen;
  const busy = busyTarget === targetKey && pending.current?.generation === latest.current.generation;

  const execute = useCallback((command: ComposerCommand) => {
    const owner = { ...latest.current };
    if (pending.current?.targetKey === owner.targetKey && pending.current.generation === owner.generation) return;
    if (command.disabledReason !== undefined) {
      Message.warning(command.disabledReason);
      return;
    }
    if (!attempt.current || attempt.current.targetKey !== owner.targetKey || attempt.current.generation !== owner.generation
      || attempt.current.version !== owner.version || attempt.current.commandId !== command.id) {
      attempt.current = { targetKey: owner.targetKey, generation: owner.generation, version: owner.version,
        commandId: command.id, key: uuidv7() };
    }
    const actionKey = attempt.current.key;
    const flight = { targetKey: owner.targetKey, generation: owner.generation };
    pending.current = flight;
    setBusyTarget(owner.targetKey);
    const isCurrent = () => active.current && latest.current.targetKey === owner.targetKey
      && latest.current.generation === owner.generation;
    void Promise.resolve().then(() => {
      // A click can be followed by navigation before its microtask runs.
      if (isCurrent()) return command.execute({ isCurrent, idempotencyKey: actionKey });
    }).then(() => {
      if (isCurrent() && latest.current.version === owner.version && latest.current.input === owner.input) {
        latest.current.setInput('');
        setDismissed(true);
      }
    }).catch(error => {
      if (!isCurrent()) return;
      if (latest.current.onError) latest.current.onError(error);
      else Message.error(`${t('messages.commands.failed')}: ${parseError(error)}`);
    }).finally(() => {
      if (pending.current === flight) {
        pending.current = undefined;
        if (isCurrent()) setBusyTarget(undefined);
      }
    });
  }, [t]);

  const resolved = resolveComposerCommand(input, commands);
  const hasCommandInput = Boolean(resolved || menuOpen || panel?.isOpen);
  const dispatchInput = useCallback(() => {
    if (panel?.isOpen) return true;
    const found = resolveComposerCommand(latest.current.input, commands);
    if (found) {
      if (found.hasArguments) Message.warning(t('messages.commands.noArguments', { command: `/${found.command.name}` }));
      else execute(found.command);
      return true;
    }
    if (menuOpen && filtered[safeIndex]) { execute(filtered[safeIndex]); return true; }
    return false;
  }, [commands, execute, filtered, menuOpen, panel?.isOpen, safeIndex, t]);

  const onKeyDown = useCallback((event: KeyboardEvent) => {
    if (isImeComposingKey(event, { composing: false, justComposed: false }) || isSystemTextEditingShortcut(event)) return false;
    if (panel?.onKeyDown(event)) return true;
    if (!menuOpen) return false;
    if (event.key === 'Escape') { event.preventDefault(); setDismissed(true); return true; }
    if (event.key === 'ArrowDown' || event.key === 'ArrowUp') {
      event.preventDefault();
      setActiveIndex((safeIndex + (event.key === 'ArrowDown' ? 1 : filtered.length - 1)) % filtered.length);
      return true;
    }
    if (event.key === 'Enter' && !event.shiftKey) { event.preventDefault(); return dispatchInput(); }
    return false;
  }, [dispatchInput, filtered.length, menuOpen, panel, safeIndex]);
  const dismiss = useCallback(() => { setDismissed(true); panel?.close(); }, [panel]);
  useEffect(() => {
    if (input && panel?.isOpen && !busy) panel.close();
  }, [input, panel?.isOpen, busy, panel?.close]);
  const isOpen = menuOpen || Boolean(panel?.isOpen);
  const menu = isOpen ? <ComposerCommandOverlay anchorRef={anchorRef}>
    {panel?.isOpen ? panel.node : <SlashCommandMenu
      title={t('messages.slash.title')} hint={t('messages.slash.hint')}
      items={filtered.map(command => ({ key: command.id, label: `/${command.name}`, description: command.description,
        disabledReason: command.disabledReason }))}
      activeIndex={safeIndex} loading={busy} onHoverItem={setActiveIndex}
      onSelectItem={item => { const command = filtered.find(candidate => candidate.id === item.key); if (command) execute(command); }}
      emptyText={t('messages.slash.empty')} />}
  </ComposerCommandOverlay> : null;
  return { menu, isOpen, busy, hasCommandInput, dispatchInput, onKeyDown, dismiss };
}
