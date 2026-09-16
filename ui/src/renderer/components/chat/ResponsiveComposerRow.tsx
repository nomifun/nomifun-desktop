import { useLayoutEffect, useState, type HTMLAttributes } from 'react';
import styles from './ResponsiveComposerRow.module.css';

/** Measure the complete toolbar, including fixed upload/send buttons and gaps. */
export default function ResponsiveComposerRow({ className = '', ...props }: HTMLAttributes<HTMLDivElement>) {
  const [element, setElement] = useState<HTMLDivElement | null>(null);
  useLayoutEffect(() => {
    if (!element) return;
    let frame = 0;
    let disposed = false;
    const measure = () => {
      if (!element.clientWidth) return;
      // Measure expanded content before paint, even while an icon is hovered.
      element.dataset.measuring = 'true';
      const compact = element.scrollWidth > element.clientWidth + 1;
      delete element.dataset.measuring;
      element.dataset.compact = String(compact);
    };
    const schedule = () => {
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(measure);
    };
    measure();
    const resize = new ResizeObserver(schedule);
    resize.observe(element);
    const changes = new MutationObserver(schedule);
    changes.observe(element, { childList: true, characterData: true, subtree: true });
    void document.fonts?.ready.then(() => { if (!disposed) schedule(); });
    return () => { disposed = true; cancelAnimationFrame(frame); resize.disconnect(); changes.disconnect(); };
  }, [element]);
  return <div {...props} ref={setElement} className={`${styles.row} ${className}`} />;
}
