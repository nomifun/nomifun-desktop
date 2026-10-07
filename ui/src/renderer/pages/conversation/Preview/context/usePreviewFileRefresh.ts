import { useCallback, useEffect, useRef } from 'react';
import type { Dispatch, MutableRefObject, SetStateAction } from 'react';
import type { PreviewTab } from './PreviewContext';
import { LARGE_TEXT_PREVIEW_MAX_LENGTH, LARGE_TEXT_PREVIEW_THRESHOLD } from '../constants';

export type PreviewFileUpdate = { file_path: string; content?: string; operation: 'write' | 'delete' };
type Ticket = { tab: PreviewTab };
type Reader = (path: string, workspace?: string) => Promise<string | null | undefined>;

/** Preserve case-sensitive names; normalize only Windows path spelling. */
export function previewFileKey(path?: string): string {
  if (!path) return '';
  if (!/^[a-z]:[\\/]/i.test(path) && !path.startsWith('\\\\') && !path.startsWith('//?/')) return path;
  let value = path.replace(/\\/g, '/').replace(/^\/\/\?\/UNC\//i, '//').replace(/^\/\/\?\//, '');
  value = value.startsWith('//') ? `//${value.slice(2).replace(/\/+/g, '/')}` : value.replace(/\/+/g, '/');
  return value.replace(/^[a-z]:/i, (drive) => drive.toUpperCase());
}

const readable = (tab: PreviewTab) => !tab.metadata?.knowledge_resource
  && ['markdown', 'html', 'code', 'image'].includes(tab.content_type);

export function usePreviewFileRefresh({ tabs, setTabs, saving, mtimes, closeTab, io }: {
  tabs: PreviewTab[];
  setTabs: Dispatch<SetStateAction<PreviewTab[]>>;
  saving: MutableRefObject<Set<string>>;
  mtimes: MutableRefObject<Map<string, number>>;
  closeTab: MutableRefObject<(id: string) => void>;
  io: {
    subscribe: (handler: (event: PreviewFileUpdate) => void) => () => void;
    metadata: (path: string, workspace?: string) => Promise<{ lastModified: number } | null | undefined>;
    text: Reader;
    image: Reader;
  };
}) {
  const currentTabs = useRef(tabs);
  currentTabs.current = tabs;
  const tickets = useRef(new Map<string, Ticket>());
  const pending = useRef(new Set<Ticket>());
  const timers = useRef(new Map<string, ReturnType<typeof setTimeout>>());
  const alive = useRef(true);

  const cancel = useCallback((id?: string) => {
    if (id) {
      tickets.current.delete(id);
      const timer = timers.current.get(id);
      if (timer) clearTimeout(timer);
      timers.current.delete(id);
    } else {
      tickets.current.clear();
      timers.current.forEach(clearTimeout);
      timers.current.clear();
    }
  }, []);

  const accepts = useCallback((ticket: Ticket, tab: PreviewTab) => alive.current
    && tickets.current.get(tab.id) === ticket
    && tab.metadata?.file_path === ticket.tab.metadata?.file_path
    && tab.metadata?.workspace === ticket.tab.metadata?.workspace
    && tab.content_type === ticket.tab.content_type && readable(tab)
    && tab.content === ticket.tab.content && tab.originalContent === ticket.tab.originalContent
    && !tab.isDirty && !saving.current.has(tab.metadata?.file_path ?? ''), [saving]);

  const run = useCallback(async (ticket: Ticket, poll: boolean, inline?: string) => {
    const tab = ticket.tab;
    const path = tab.metadata?.file_path;
    if (!path) return;
    let nextMtime: number | undefined;
    try {
      if (poll) {
        const metadata = await io.metadata(path, tab.metadata?.workspace);
        if (!metadata) throw new Error('Preview file metadata is unavailable');
        const current = currentTabs.current.find((item) => item.id === tab.id);
        if (!current || !accepts(ticket, current)) return;
        const previous = mtimes.current.get(path);
        if (previous === undefined) { mtimes.current.set(path, metadata.lastModified); return; }
        if (previous === metadata.lastModified) return;
        nextMtime = metadata.lastModified;
      }
      setTabs((latest) => latest.map((item) => accepts(ticket, item) ? { ...item, fileRefreshing: true } : item));
      const content = tab.content_type === 'image'
        ? await io.image(path, tab.metadata?.workspace)
        : typeof inline === 'string' ? inline : await io.text(path, tab.metadata?.workspace);
      if (typeof content !== 'string') throw new Error('Preview file content is unavailable');
      const truncated = tab.content_type === 'code' && content.length > LARGE_TEXT_PREVIEW_THRESHOLD;
      const visibleContent = truncated ? content.slice(0, LARGE_TEXT_PREVIEW_MAX_LENGTH) : content;
      setTabs((latest) => latest.map((item) => {
        if (!accepts(ticket, item)) return item;
        if (nextMtime !== undefined) mtimes.current.set(path, nextMtime);
        return { ...item, content: visibleContent, originalContent: visibleContent, isDirty: false,
          fileReadError: false, fileRefreshing: false,
          metadata: { ...item.metadata, truncated, ...(truncated ? { editable: false } : {}) } };
      }));
    } catch (error) {
      const current = currentTabs.current.find((item) => item.id === tab.id);
      if (current && accepts(ticket, current)) {
        console.error('[PreviewContext] File reconciliation failed:', path, error);
        setTabs((latest) => latest.map((item) => accepts(ticket, item) ? { ...item, fileReadError: true, fileRefreshing: false } : item));
      }
    } finally {
      // State updaters can run after this microtask; retain the completed ticket
      // until the next request/edit, and track in-flight work separately below.
      pending.current.delete(ticket);
    }
  }, [accepts, io, mtimes, setTabs]);
  const refresh = useCallback(async (id: string, poll = false) => {
    const tab = currentTabs.current.find((item) => item.id === id);
    if (!tab?.metadata?.file_path || !readable(tab) || tab.isDirty || saving.current.has(tab.metadata.file_path)) return;
    const old = tickets.current.get(id);
    if (poll && old && (pending.current.has(old) || timers.current.has(id))) return;
    cancel(id);
    const ticket = { tab };
    tickets.current.set(id, ticket);
    pending.current.add(ticket);
    await run(ticket, poll);
  }, [cancel, run, saving]);

  useEffect(() => {
    alive.current = true;
    const unsubscribe = io.subscribe((event) => {
      const key = previewFileKey(event.file_path);
      if (!key) return;
      for (const tab of currentTabs.current.filter((item) => previewFileKey(item.metadata?.file_path) === key)) {
        cancel(tab.id);
        if (event.operation === 'delete') {
          if (tab.isDirty || saving.current.has(tab.metadata?.file_path ?? '')) {
            setTabs((latest) => latest.map((item) => item.id === tab.id ? { ...item, fileReadError: true, fileRefreshing: false } : item));
          } else { closeTab.current(tab.id); }
          continue;
        }
        // Office/PDF and managed knowledge previews keep their own readers.
        if (!readable(tab) || tab.isDirty || saving.current.has(tab.metadata?.file_path ?? '')) continue;
        const ticket = { tab };
        tickets.current.set(tab.id, ticket);
        timers.current.set(tab.id, setTimeout(() => {
          timers.current.delete(tab.id);
          const current = currentTabs.current.find((item) => item.id === tab.id);
          if (!current || !accepts(ticket, current)) return;
          pending.current.add(ticket);
          void run(ticket, false, event.content);
        }, 500));
      }
    });
    return () => { alive.current = false; cancel(); pending.current.clear(); unsubscribe(); };
  }, [accepts, cancel, closeTab, io, run, saving, setTabs]);

  return { cancel, refresh };
}
