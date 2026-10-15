import type { KeyboardEvent, ReactNode } from 'react';

/** A target advertises only actions it actually owns. Disabled actions cannot
 * accidentally fall through to another domain's handler. */
export type ComposerCommand = {
  id: string;
  name: string;
  description: string;
} & (
  | { disabledReason?: undefined; execute: (execution: ComposerCommandExecution) => void | Promise<void> }
  | { disabledReason: string; execute?: never }
);

export type ComposerCommandExecution = { isCurrent: () => boolean; idempotencyKey: string };

export type ComposerCommandPanel = {
  isOpen: boolean;
  node: ReactNode;
  onKeyDown: (event: KeyboardEvent) => boolean;
  close: () => void;
};

export function commandQuery(input: string): string | null {
  const match = /^\/([^\s/]*)$/.exec(input);
  return match?.[0].length === input.length ? match[1] : null;
}

/** Match a whole user command, never slash text embedded in a prompt or path. */
export function resolveComposerCommand(input: string, commands: readonly ComposerCommand[]) {
  const match = /^\/([^\s/]+)(?:\s+([\s\S]*))?$/.exec(input.trim());
  if (!match) return null;
  const command = commands.find(item => item.name.toLowerCase() === match[1].toLowerCase());
  return command ? { command, hasArguments: Boolean(match[2]?.trim()) } : null;
}

export function validateComposerCommands(commands: readonly ComposerCommand[]): void {
  const ids = new Set<string>();
  const names = new Set<string>();
  for (const command of commands) {
    const name = command.name.toLowerCase();
    if (!command.id || !/^[^\s/]+$/.test(name) || ids.has(command.id) || names.has(name)) {
      throw new Error('Composer commands require unique identities and aliases');
    }
    ids.add(command.id);
    names.add(name);
  }
}
