import React, { createContext, useMemo, useState } from 'react';
import styles from './styles.module.css';

type ComposerTool = 'skills' | 'mcp' | 'collaboration';
export const SessionComposerToolsContext = createContext<{
  openTool: ComposerTool | undefined;
  setOpenTool: React.Dispatch<React.SetStateAction<ComposerTool | undefined>>;
} | null>(null);

/** Shared rail for composer-owned tools that do not edit the frozen Agent binding. */
export const ComposerToolRail: React.FC<{
  children: React.ReactNode;
  ariaLabel: string;
}> = ({ children, ariaLabel }) => (
  <aside className={styles.rail} data-composer-tools aria-label={ariaLabel}>
    {children}
  </aside>
);

/** Place inside the input surface so the tools share its border and focus ring. */
export const SessionCapabilityComposerLayout: React.FC<{
  children: React.ReactNode;
  picker?: React.ReactNode;
}> = ({ children, picker }) => {
  const [openTool, setOpenTool] = useState<ComposerTool>();
  const context = useMemo(() => ({ openTool, setOpenTool }), [openTool]);
  if (!picker) return <>{children}</>;
  return (
    <SessionComposerToolsContext.Provider value={context}>
      <div
        className={styles.composerLayout}
        onKeyDown={(event) => {
          if (event.key === 'Escape' && !event.defaultPrevented && openTool === 'collaboration') {
            event.preventDefault();
            setOpenTool(undefined);
            event.currentTarget.querySelector<HTMLButtonElement>('[data-composer-tool-trigger="collaboration"]')?.focus();
          }
        }}
        onPointerDown={(event) => {
          // Portaled tool panels bubble through React but are outside this DOM node.
          if (
            event.target instanceof Element &&
            event.currentTarget.contains(event.target) &&
            !event.target.closest('[data-composer-tools]')
          ) {
            setOpenTool(undefined);
          }
        }}
      >
        <div className={styles.composerMain}>{children}</div>
        {picker}
      </div>
    </SessionComposerToolsContext.Provider>
  );
};
