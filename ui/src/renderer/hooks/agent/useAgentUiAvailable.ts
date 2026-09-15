import { createContext, useContext } from 'react';

/** Seeded by authenticated bootstrap before any Session renderer mounts. */
export const AgentUiAvailabilityContext = createContext(false);

export function useAgentUiAvailable() {
  return useContext(AgentUiAvailabilityContext);
}
