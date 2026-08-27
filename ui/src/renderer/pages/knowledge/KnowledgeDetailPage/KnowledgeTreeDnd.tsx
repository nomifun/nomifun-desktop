import classNames from 'classnames';
import React, {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';
import {
  DndContext,
  DragOverlay,
  KeyboardSensor,
  MouseSensor,
  TouchSensor,
  pointerWithin,
  rectIntersection,
  useDraggable,
  useDroppable,
  useSensor,
  useSensors,
  type CollisionDetection,
  type DraggableAttributes,
  type DraggableSyntheticListeners,
  type DragEndEvent,
  type DragOverEvent,
  type DragStartEvent,
} from '@dnd-kit/core';
import { FileFocus, FolderOpen } from '@icon-park/react';
import type { IKnowledgeTreeEntry } from '@/common/adapter/ipcBridge';
import type { KnowledgeRelocationIssue } from './treeModel';
import { hasKnowledgeEntryCapability } from './entryCapabilities';
import {
  KNOWLEDGE_ROOT_DROP_ID,
  knowledgeDragData,
  knowledgeDragEntryFromData,
  knowledgeDropData,
  knowledgeDropTargetFromData,
  knowledgeRootDropData,
  knowledgeTreeDragId,
  knowledgeTreeDropId,
  resolveKnowledgeDrop,
  type KnowledgeDropTarget,
} from './knowledgeTreeDndModel';

const knowledgeTreeCollisionDetection: CollisionDetection = (args) => {
  // `pointerWithin` gives the most intuitive nested row/root result for mouse
  // and touch. Keyboard drags have no pointer coordinates, so fall back to
  // rectangle intersections instead of making keyboard movement a no-op.
  const collisions = pointerWithin(args);
  const candidates = collisions.length > 0 ? collisions : rectIntersection(args);
  const nodeCollision = candidates.find((collision) => collision.id !== KNOWLEDGE_ROOT_DROP_ID);
  if (nodeCollision) return [nodeCollision];
  const rootCollision = candidates.find((collision) => collision.id === KNOWLEDGE_ROOT_DROP_ID);
  return rootCollision ? [rootCollision] : [];
};

type KnowledgeTreeDndLabels = {
  dropHint: string;
  invalidTarget: string;
  rootFolder: string;
  describeIssue: (issue: KnowledgeRelocationIssue) => string;
  moveTo: (folder: string) => string;
};

type KnowledgeTreeDndProps = {
  children: React.ReactNode;
  disabled: boolean;
  expandedDirectoryPaths: string[];
  labels: KnowledgeTreeDndLabels;
  onExpandDirectory: (path: string) => void;
  onInvalidDrop: (issue: KnowledgeRelocationIssue) => void;
  onLoadDirectory: (entry: IKnowledgeTreeEntry) => Promise<void>;
  onLoadError: (error: unknown) => void;
  onRelocate: (entry: IKnowledgeTreeEntry, destinationParentPath: string) => void;
};

type KnowledgeTreeDndContextValue = {
  activeEntry: IKnowledgeTreeEntry | null;
  disabled: boolean;
};

const KnowledgeTreeDndContext = createContext<KnowledgeTreeDndContextValue | null>(null);

type KnowledgeTreeDragRowContextValue = {
  attributes: DraggableAttributes;
  listeners: DraggableSyntheticListeners;
  setActivatorNodeRef: (node: HTMLElement | null) => void;
};

const KnowledgeTreeDragRowContext = createContext<KnowledgeTreeDragRowContextValue | null>(null);

function useKnowledgeTreeDndContext(): KnowledgeTreeDndContextValue {
  const value = useContext(KnowledgeTreeDndContext);
  if (!value) throw new Error('KnowledgeTreeDndRow must be rendered inside KnowledgeTreeDnd');
  return value;
}

function useKnowledgeTreeDragRowContext(): KnowledgeTreeDragRowContextValue {
  const value = useContext(KnowledgeTreeDragRowContext);
  if (!value) throw new Error('KnowledgeTreeDndHandle must be rendered inside KnowledgeTreeDndRow');
  return value;
}

const KnowledgeTreeRootDropArea: React.FC<{
  activeEntry: IKnowledgeTreeEntry | null;
  children: React.ReactNode;
  disabled: boolean;
}> = ({ activeEntry, children, disabled }) => {
  const { setNodeRef, isOver } = useDroppable({
    id: KNOWLEDGE_ROOT_DROP_ID,
    data: knowledgeRootDropData(),
    disabled,
  });
  const decision = activeEntry
    ? resolveKnowledgeDrop(activeEntry, knowledgeRootDropData())
    : null;

  return (
    <div
      ref={setNodeRef}
      className={classNames(
        'knowledge-tree-root-drop min-h-full rounded-8px transition-colors',
        isOver && decision?.accepted &&
          'bg-[rgba(var(--primary-6),0.07)] ring-1 ring-inset ring-[rgba(var(--primary-6),0.28)]'
      )}
    >
      {children}
    </div>
  );
};

export const KnowledgeTreeDndRow: React.FC<{
  children: React.ReactNode;
  item: IKnowledgeTreeEntry;
}> = ({ children, item }) => {
  const { activeEntry, disabled } = useKnowledgeTreeDndContext();
  const drag = useDraggable({
    id: knowledgeTreeDragId(item),
    data: knowledgeDragData(item),
    disabled: disabled || !hasKnowledgeEntryCapability(item, 'relocate'),
  });
  // Files deliberately register as non-accepting droppables. Otherwise the
  // encompassing root area wins while the pointer is visibly over a file row.
  const drop = useDroppable({
    id: knowledgeTreeDropId(item),
    data: knowledgeDropData(item),
    // Keep non-accepting rows registered so dropping over a file or restricted
    // directory cannot accidentally fall through to the encompassing root.
    disabled,
  });
  const setNodeRef = useCallback(
    (node: HTMLElement | null) => {
      drag.setNodeRef(node);
      drop.setNodeRef(node);
    },
    [drag.setNodeRef, drop.setNodeRef]
  );
  const decision = activeEntry
    ? resolveKnowledgeDrop(activeEntry, knowledgeDropData(item))
    : null;
  const validTarget = drop.isOver && decision?.accepted;
  const invalidTarget = drop.isOver && activeEntry && decision && !decision.accepted;
  const rowContext = useMemo<KnowledgeTreeDragRowContextValue>(
    () => ({
      attributes: drag.attributes,
      listeners: drag.listeners,
      setActivatorNodeRef: drag.setActivatorNodeRef,
    }),
    [drag.attributes, drag.listeners, drag.setActivatorNodeRef]
  );

  return (
    <KnowledgeTreeDragRowContext.Provider value={rowContext}>
      <div
        ref={setNodeRef}
        data-knowledge-path={item.rel_path}
        className={classNames(
          'knowledge-tree-node-drag-shell w-full rounded-6px transition-colors',
          drag.isDragging && 'opacity-35',
          validTarget &&
            'bg-[rgba(var(--primary-6),0.16)] ring-1 ring-inset ring-[rgba(var(--primary-6),0.42)]',
          invalidTarget &&
            'bg-[rgba(var(--danger-6),0.10)] ring-1 ring-inset ring-[rgba(var(--danger-6),0.30)]'
        )}
      >
        {children}
      </div>
    </KnowledgeTreeDragRowContext.Provider>
  );
};

/** Keyboard-accessible activator kept separate from the row's action menu. */
export const KnowledgeTreeDndHandle: React.FC<{
  'aria-label': string;
  children: React.ReactNode;
  className?: string;
}> = ({ children, className, ...props }) => {
  const { attributes, listeners, setActivatorNodeRef } = useKnowledgeTreeDragRowContext();
  return (
    <span
      ref={setActivatorNodeRef}
      {...attributes}
      {...listeners}
      {...props}
      className={className}
      style={{ touchAction: 'pan-y' }}
    >
      {children}
    </span>
  );
};

export const KnowledgeTreeDnd: React.FC<KnowledgeTreeDndProps> = ({
  children,
  disabled,
  expandedDirectoryPaths,
  labels,
  onExpandDirectory,
  onInvalidDrop,
  onLoadDirectory,
  onLoadError,
  onRelocate,
}) => {
  const [activeEntry, setActiveEntry] = useState<IKnowledgeTreeEntry | null>(null);
  const [overTarget, setOverTarget] = useState<KnowledgeDropTarget | null>(null);
  const hoverExpandTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);
  const hoverExpandPathRef = useRef<string | null>(null);
  const sensors = useSensors(
    useSensor(MouseSensor, { activationConstraint: { distance: 5 } }),
    useSensor(TouchSensor, { activationConstraint: { delay: 250, tolerance: 5 } }),
    useSensor(KeyboardSensor)
  );

  const clearHoverExpandTimer = useCallback(() => {
    if (hoverExpandTimerRef.current) clearTimeout(hoverExpandTimerRef.current);
    hoverExpandTimerRef.current = null;
    hoverExpandPathRef.current = null;
  }, []);

  useEffect(() => clearHoverExpandTimer, [clearHoverExpandTimer]);

  const finishDrag = useCallback(() => {
    clearHoverExpandTimer();
    setActiveEntry(null);
    setOverTarget(null);
  }, [clearHoverExpandTimer]);

  const handleDragStart = useCallback((event: DragStartEvent) => {
    setActiveEntry(knowledgeDragEntryFromData(event.active.data.current));
    setOverTarget(null);
  }, []);

  const handleDragOver = useCallback(
    (event: DragOverEvent) => {
      const source = knowledgeDragEntryFromData(event.active.data.current);
      const target = knowledgeDropTargetFromData(event.over?.data.current);
      setOverTarget(target);

      const directory = target?.entry;
      const decision = source && target ? resolveKnowledgeDrop(source, target) : null;
      if (
        !source ||
        !directory?.is_dir ||
        !decision?.accepted ||
        expandedDirectoryPaths.includes(directory.rel_path)
      ) {
        clearHoverExpandTimer();
        return;
      }
      if (hoverExpandPathRef.current === directory.rel_path) return;

      clearHoverExpandTimer();
      hoverExpandPathRef.current = directory.rel_path;
      hoverExpandTimerRef.current = setTimeout(() => {
        hoverExpandTimerRef.current = null;
        hoverExpandPathRef.current = null;
        void onLoadDirectory(directory)
          .then(() => onExpandDirectory(directory.rel_path))
          .catch(onLoadError);
      }, 600);
    },
    [
      clearHoverExpandTimer,
      expandedDirectoryPaths,
      onExpandDirectory,
      onLoadDirectory,
      onLoadError,
    ]
  );

  const handleDragEnd = useCallback(
    (event: DragEndEvent) => {
      const source = knowledgeDragEntryFromData(event.active.data.current);
      const target = knowledgeDropTargetFromData(event.over?.data.current);
      finishDrag();
      if (!source) return;
      const decision = resolveKnowledgeDrop(source, target);
      if (!decision.accepted) {
        if (decision.issue !== 'same-parent' && decision.issue !== 'invalid-target') {
          onInvalidDrop(decision.issue);
        }
        return;
      }
      onRelocate(source, decision.destinationParentPath);
    },
    [finishDrag, onInvalidDrop, onRelocate]
  );

  const decision = activeEntry ? resolveKnowledgeDrop(activeEntry, overTarget) : null;
  const overlayStatus = !overTarget
    ? labels.dropHint
    : !decision?.accepted
      ? decision?.issue === 'invalid-target'
        ? labels.invalidTarget
        : decision
          ? labels.describeIssue(decision.issue)
          : labels.dropHint
      : labels.moveTo(overTarget.destinationParentPath || labels.rootFolder);
  const invalidOverTarget = Boolean(overTarget && decision && !decision.accepted);
  const contextValue = useMemo(
    () => ({ activeEntry, disabled }),
    [activeEntry, disabled]
  );

  return (
    <DndContext
      sensors={sensors}
      collisionDetection={knowledgeTreeCollisionDetection}
      autoScroll
      onDragStart={handleDragStart}
      onDragOver={handleDragOver}
      onDragCancel={finishDrag}
      onDragEnd={handleDragEnd}
    >
      <KnowledgeTreeDndContext.Provider value={contextValue}>
        <KnowledgeTreeRootDropArea disabled={disabled} activeEntry={activeEntry}>
          {children}
        </KnowledgeTreeRootDropArea>
        <DragOverlay dropAnimation={null}>
          {activeEntry ? (
            <div
              className={classNames(
                'max-w-260px rounded-8px border border-solid bg-[var(--color-bg-3)] px-10px py-8px shadow-lg',
                invalidOverTarget
                  ? 'border-[rgba(var(--danger-6),0.45)]'
                  : 'border-[rgba(var(--primary-6),0.42)]'
              )}
            >
              <div className='flex min-w-0 items-center gap-6px text-12px font-600 text-[var(--color-text-1)]'>
                {activeEntry.is_dir ? (
                  <FolderOpen theme='outline' size='14' />
                ) : (
                  <FileFocus theme='outline' size='14' />
                )}
                <span className='truncate'>{activeEntry.name}</span>
              </div>
              <div
                className={classNames(
                  'mt-3px truncate text-10px',
                  invalidOverTarget ? 'text-danger-6' : 'text-[var(--color-text-3)]'
                )}
              >
                {overlayStatus}
              </div>
            </div>
          ) : null}
        </DragOverlay>
      </KnowledgeTreeDndContext.Provider>
    </DndContext>
  );
};
