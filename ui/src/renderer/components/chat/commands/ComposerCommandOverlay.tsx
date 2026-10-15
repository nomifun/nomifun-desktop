import { autoUpdate, flip, FloatingPortal, offset, shift, size, useFloating } from '@floating-ui/react';
import { useEffect, type ReactNode, type RefObject } from 'react';

/** One viewport-aware surface for commands and their follow-up panels. A
 * portal keeps embedded application/Canvas panes from clipping the menu. */
export function ComposerCommandOverlay({ anchorRef, children }: {
  anchorRef: RefObject<HTMLElement | null>;
  children: ReactNode;
}) {
  const { refs, floatingStyles, isPositioned } = useFloating({
    placement: 'top-start',
    strategy: 'fixed',
    whileElementsMounted: autoUpdate,
    middleware: [offset(8), flip({ padding: 12 }), shift({ padding: 12 }), size({
      padding: 12,
      apply({ availableHeight, availableWidth, rects, elements }) {
        Object.assign(elements.floating.style, {
          width: `${Math.max(0, Math.min(rects.reference.width, availableWidth))}px`,
          maxHeight: `${Math.max(0, availableHeight)}px`,
        });
      },
    })],
  });
  // A restored slash draft can mount this child before the parent's DOM ref
  // attaches. Bind after the commit, and never paint the unpositioned portal.
  useEffect(() => { refs.setReference(anchorRef.current); }, [anchorRef, refs.setReference]);
  return <FloatingPortal><div ref={refs.setFloating} style={{ ...floatingStyles, zIndex: 1100, overflowY: 'auto', visibility: isPositioned ? 'visible' : 'hidden' }}
    data-composer-command-overlay>{children}</div></FloatingPortal>;
}
