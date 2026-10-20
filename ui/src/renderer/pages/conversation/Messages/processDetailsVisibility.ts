import { createContext } from 'react';

/** Keep local disclosure choices while suspending payload observers in hidden parents. */
export const ProcessDetailsVisibilityContext = createContext(true);
