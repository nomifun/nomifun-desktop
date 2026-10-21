import { createContext, useContext, useState } from 'react';

/** Reading choices belong to the mounted conversation, not a process wrapper.
 * A newly loaded steering boundary may move the same canonical row between
 * wrappers; the choice must survive that presentation-only move. */
export const ProcessExpansionContext = createContext<Map<string, boolean> | null>(null);

export function useProcessExpansion(identity: string): [boolean | undefined, (expanded: boolean) => void] {
  const memory = useContext(ProcessExpansionContext);
  const [choice, setChoice] = useState(() => ({ memory, identity, expanded: memory?.get(identity) }));
  const expanded = choice.memory === memory && choice.identity === identity ? choice.expanded : memory?.get(identity);
  return [expanded, next => {
    memory?.set(identity, next);
    setChoice({ memory, identity, expanded: next });
  }];
}
