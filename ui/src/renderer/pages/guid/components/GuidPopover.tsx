import {
  autoUpdate, flip, FloatingFocusManager, FloatingPortal, offset, shift,
  size, useClick, useDismiss, useFloating, useInteractions, useRole,
} from '@floating-ui/react';
import { type ReactNode, useLayoutEffect } from 'react';
import styles from './GuidCompanionShowcase.module.css';

/** Keyboard/focus-aware desktop popover shared by the home controls. */
export default function GuidPopover({ open, onOpenChange, label, trigger, children, triggerClassName, panelClassName, placement = 'bottom-end', pressed, anchorToFigure = false, portal = true, initialFocus = 0 }: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
  label: string;
  trigger: ReactNode;
  children: ReactNode;
  triggerClassName?: string;
  panelClassName?: string;
  placement?: 'top' | 'bottom-end' | 'right-start';
  pressed?: boolean;
  anchorToFigure?: boolean;
  /** Nested actions stay inside their parent dialog for outside-click and focus handling. */
  portal?: boolean;
  initialFocus?: number;
}) {
  const { refs, floatingStyles, context } = useFloating({
    open, onOpenChange, placement, strategy: 'fixed', whileElementsMounted: autoUpdate,
    middleware: [offset(anchorToFigure ? { mainAxis: 10, crossAxis: -42 } : 10), flip({ padding: 12 }), shift({ padding: 12 }), size({
      padding: 12,
      apply({ availableHeight, elements }) {
        elements.floating.style.maxHeight = `${Math.max(0, Math.min(460, availableHeight))}px`;
      },
    })],
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
      className={`${styles.popover} ${panelClassName ?? ''}`}
      aria-label={label} {...getFloatingProps({
        onKeyDown(event) {
          if (!portal && event.key === 'Escape') {
            event.stopPropagation();
            onOpenChange(false);
          }
        },
      })}>{children}</div>
  </FloatingFocusManager>;
  return <>
    <button type='button' ref={refs.setReference} className={triggerClassName}
      aria-label={label} aria-pressed={pressed} {...getReferenceProps()}>{trigger}</button>
    {open && (portal ? <FloatingPortal>{panel}</FloatingPortal> : panel)}
  </>;
}
