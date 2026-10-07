import {
  arrow, autoUpdate, flip, FloatingArrow, FloatingFocusManager, FloatingPortal, offset, shift,
  size, useClick, useDismiss, useFloating, useInteractions, useRole,
} from '@floating-ui/react';
import { type ReactNode, useLayoutEffect, useRef } from 'react';
import styles from './GuidCompanionShowcase.module.css';

/** Keyboard/focus-aware desktop popover shared by the home controls. */
export default function GuidPopover({ open, onOpenChange, label, trigger, children, triggerClassName, panelClassName, placement = 'bottom-end', pressed, anchorToFigure = false, showArrow = false, portal = true, initialFocus = 0 }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  label: string;
  trigger: ReactNode;
  children: ReactNode;
  triggerClassName?: string;
  panelClassName?: string;
  placement?: 'top' | 'bottom-end';
  pressed?: boolean;
  anchorToFigure?: boolean;
  showArrow?: boolean;
  /** Nested actions stay inside their parent dialog for outside-click and focus handling. */
  portal?: boolean;
  initialFocus?: number;
}) {
  const arrowRef = useRef<SVGSVGElement>(null);
  const { refs, floatingStyles, context } = useFloating({
    open, onOpenChange, placement, strategy: 'fixed', whileElementsMounted: autoUpdate,
    middleware: [offset(showArrow ? 12 : 10), flip({ padding: 12 }), shift({ padding: 12 }), size({
      padding: 12,
      apply({ availableHeight, elements }) {
        elements.floating.style.maxHeight = `${Math.max(0, Math.min(460, availableHeight))}px`;
      },
    }), showArrow && arrow({ element: arrowRef, padding: 10 })],
  });
  useLayoutEffect(() => {
    if (anchorToFigure && open) {
      refs.setPositionReference(refs.domReference.current?.querySelector('[data-showcase-art]') ?? null);
    }
  }, [anchorToFigure, open, refs]);
  const { getReferenceProps, getFloatingProps } = useInteractions([
    useClick(context), useDismiss(context), useRole(context, { role: 'dialog' }),
  ]);
  const panel = open && <FloatingFocusManager context={context} modal={false} initialFocus={initialFocus}>
    <div ref={refs.setFloating} style={floatingStyles}
      className={`${styles.popover} ${showArrow ? styles.pointedPopover : ''} ${panelClassName ?? ''}`}
      aria-label={label} {...getFloatingProps({
        onKeyDown(event) {
          if (!portal && event.key === 'Escape') {
            event.stopPropagation();
            onOpenChange(false);
          }
        },
      })}>
      {showArrow ? <>
        <div className={styles.popoverContent}>{children}</div>
        <FloatingArrow ref={arrowRef} context={context} tipRadius={1}
          fill='var(--bg-base)' stroke='var(--color-border-2)' strokeWidth={1} />
      </> : children}
    </div>
  </FloatingFocusManager>;
  return <>
    <button type='button' ref={refs.setReference} className={triggerClassName}
      aria-label={label} aria-pressed={pressed} {...getReferenceProps()}>{trigger}</button>
    {open && (portal ? <FloatingPortal>{panel}</FloatingPortal> : panel)}
  </>;
}
