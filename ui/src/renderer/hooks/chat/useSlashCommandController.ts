import type { SlashCommandItem } from '@/common/chat/slash/types';
import { useCallback, useEffect, useMemo, useState } from 'react';
import type { KeyboardEvent as ReactKeyboardEvent } from 'react';

// Command names are a single token, including exact namespaced Skill IDs
// (for example /skill:acme.review). This only filters the server's catalog;
// it does not resolve an identity or authorize command execution.
const SLASH_QUERY_RE = /^\/([^\s]*)$/;

export function matchSlashQuery(input: string): string | null {
  const match = input.match(SLASH_QUERY_RE);
  return match && match[0].length === input.length ? match[1] : null;
}

function getSelectionBehavior(command: SlashCommandItem): 'execute' | 'insert' {
  if (command.selectionBehavior) {
    return command.selectionBehavior;
  }
  return command.kind === 'builtin' ? 'execute' : 'insert';
}

interface UseSlashCommandControllerOptions {
  input: string;
  commands: SlashCommandItem[];
  onExecuteBuiltin?: (name: string) => void;
  onSelectTemplate?: (name: string) => void;
}

export function useSlashCommandController(options: UseSlashCommandControllerOptions) {
  const { input, commands, onExecuteBuiltin, onSelectTemplate } = options;
  const query = useMemo(() => matchSlashQuery(input), [input]);
  const [activeIndex, setActiveIndex] = useState(0);
  const [dismissed, setDismissed] = useState(false);

  // Reset state only when query changes, not when commands array updates.
  // This prevents dropdown from reopening when the agent dynamically adds commands
  // while the user is typing.
  useEffect(() => {
    setActiveIndex(0);
    setDismissed(false);
  }, [query]);

  const filteredCommands = useMemo(() => {
    if (query === null) {
      return [];
    }
    const keyword = query.trim().toLowerCase();
    if (!keyword) {
      return commands;
    }
    return commands.filter((command) => command.name.toLowerCase().startsWith(keyword));
  }, [commands, query]);

  const isOpen = query !== null && !dismissed && filteredCommands.length > 0;

  const executeCommand = useCallback(
    (index: number) => {
      const command = filteredCommands[index];
      if (!command) {
        return false;
      }
      if (getSelectionBehavior(command) === 'insert') {
        onSelectTemplate?.(command.name);
      } else if (command.kind === 'builtin') {
        onExecuteBuiltin?.(command.name);
      } else {
        onSelectTemplate?.(command.name);
      }
      setDismissed(true);
      return true;
    },
    [filteredCommands, onExecuteBuiltin, onSelectTemplate]
  );

  const onKeyDown = useCallback(
    (event: ReactKeyboardEvent) => {
      if (!isOpen) {
        return false;
      }

      if (event.key === 'Escape') {
        event.preventDefault();
        setDismissed(true);
        return true;
      }

      if (event.key === 'ArrowDown') {
        event.preventDefault();
        setActiveIndex((prev) => (prev + 1) % filteredCommands.length);
        return true;
      }

      if (event.key === 'ArrowUp') {
        event.preventDefault();
        setActiveIndex((prev) => (prev - 1 + filteredCommands.length) % filteredCommands.length);
        return true;
      }

      if (event.key === 'Enter' && !event.shiftKey) {
        event.preventDefault();
        return executeCommand(activeIndex);
      }

      return false;
    },
    [activeIndex, executeCommand, filteredCommands.length, isOpen]
  );

  return {
    isOpen,
    activeIndex,
    filteredCommands,
    onKeyDown,
    onSelectByIndex: executeCommand,
    setDismissed,
    setActiveIndex,
  };
}
