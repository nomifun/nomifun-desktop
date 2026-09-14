import type { IMcpServer } from '@/common/config/storage';
import { resolveSkillDisplay } from '@/renderer/pages/settings/skill/skillDisplay';
import { Button, Checkbox, Spin, Tooltip } from '@arco-design/web-react';
import { autoUpdate, flip, FloatingFocusManager, FloatingPortal, offset, shift, size, useDismiss, useFloating, useInteractions, useRole } from '@floating-ui/react';
import { CheckOne, CloseSmall, Lightning, Puzzle, Right } from '@icon-park/react';
import React, { useContext, useLayoutEffect, useMemo, useRef, useState } from 'react';
import { useTranslation } from 'react-i18next';
import { useNavigate } from 'react-router-dom';
import type { SessionCapabilityCatalog, SessionCapabilityDraft, SessionSkillOption } from './model';
import styles from './styles.module.css';
import { SessionComposerToolsContext } from './ComposerLayout';

type PickerKind = 'skills' | 'mcp';
const EMPTY_LOCKED_MCP_SERVER_IDS = new Set<string>();

type SessionCapabilityPickerProps = {
  catalog: SessionCapabilityCatalog;
  draft: SessionCapabilityDraft;
  onChange: (draft: SessionCapabilityDraft) => void;
  loading?: boolean;
  loadFailed?: boolean;
  onRetry?: () => void;
  applyMode: 'create' | 'next-send';
  disabled?: boolean;
  lockedMcpServerIds?: ReadonlySet<string>;
  children?: React.ReactNode;
};

export { SessionCapabilityComposerLayout } from './ComposerLayout';

const mcpStatus = (server: IMcpServer) => {
  if (!server.enabled) return 'disabled';
  if (server.last_test_status === 'error') return 'error';
  return 'available';
};

const CapabilityCheckbox = ({ checked, disabled, onChange, label }: {
  checked: boolean;
  disabled?: boolean;
  onChange: (checked: boolean) => void;
  label: string;
}) => (
  <span className={styles.checkboxCell} onClick={(event) => event.stopPropagation()}>
    <Checkbox checked={checked} disabled={disabled} onChange={onChange}>{label}</Checkbox>
  </span>
);

const CapabilityDescription = ({ children }: { children: string }) => {
  const ref = useRef<HTMLElement>(null);
  const [truncated, setTruncated] = useState(false);
  useLayoutEffect(() => {
    const element = ref.current;
    if (!element) return;
    const measure = () => setTruncated(element.scrollWidth > element.clientWidth);
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(element);
    return () => observer.disconnect();
  }, [children]);
  return (
    <Tooltip content={children} disabled={!truncated} trigger={['hover', 'focus']} position='left' className={styles.descriptionTooltip}>
      <small ref={ref} tabIndex={truncated ? 0 : undefined}>{children}</small>
    </Tooltip>
  );
};

const SessionCapabilityPicker: React.FC<SessionCapabilityPickerProps> = ({
  catalog,
  draft,
  onChange,
  loading = false,
  loadFailed = false,
  onRetry,
  applyMode,
  disabled = false,
  lockedMcpServerIds = EMPTY_LOCKED_MCP_SERVER_IDS,
  children,
}) => {
  const { t, i18n } = useTranslation();
  const navigate = useNavigate();
  const toolsContext = useContext(SessionComposerToolsContext);
  const [localOpen, setLocalOpen] = useState<PickerKind>();
  const open = toolsContext ? toolsContext.openTool : localOpen;
  const capabilityOpen = open === 'skills' || open === 'mcp' ? open : undefined;
  const setOpen = (next: PickerKind | undefined) => {
    if (toolsContext) {
      toolsContext.setOpenTool((current) => next ?? (current === 'collaboration' ? current : undefined));
    } else {
      setLocalOpen(next);
    }
  };
  const railRef = useRef<HTMLElement>(null);
  const { refs, floatingStyles, context } = useFloating({
    open: Boolean(capabilityOpen),
    onOpenChange: (visible) => { if (!visible) setOpen(undefined); },
    placement: 'top-end',
    strategy: 'fixed',
    whileElementsMounted: autoUpdate,
    middleware: [
      offset(8),
      flip({ padding: 12 }),
      shift({ padding: 12 }),
      size({
        padding: 12,
        apply({ availableHeight, elements }) {
          elements.floating.style.maxHeight = `${Math.max(0, availableHeight)}px`;
        },
      }),
    ],
  });
  const dismiss = useDismiss(context, {
    outsidePress: (event) => !(event.target instanceof Node && railRef.current?.contains(event.target)),
  });
  const role = useRole(context, { role: 'dialog' });
  const { getReferenceProps, getFloatingProps } = useInteractions([dismiss, role]);
  const selectedSkills = useMemo(() => new Set(draft.skillNames), [draft.skillNames]);
  const selectedMcp = useMemo(() => new Set(draft.mcpServerIds), [draft.mcpServerIds]);
  const applyNote = applyMode === 'create'
    ? t('conversation.capabilityPicker.createApplyNote', { defaultValue: '新会话 · 创建时应用' })
    : t('conversation.capabilityPicker.nextSendApplyNote', { defaultValue: '本会话 · 下次发送时应用' });

  const toggleSkill = (skill: SessionSkillOption, checked: boolean) => {
    onChange({
      ...draft,
      skillNames: checked
        ? Array.from(new Set([...draft.skillNames, skill.name]))
        : draft.skillNames.filter((name) => name !== skill.name),
    });
  };
  const toggleMcp = (server: IMcpServer, checked: boolean) => {
    onChange({
      ...draft,
      mcpServerIds: checked
        ? Array.from(new Set([...draft.mcpServerIds, server.mcp_server_id]))
        : draft.mcpServerIds.filter((id) => id !== server.mcp_server_id),
    });
  };

  const panel = (kind: PickerKind) => {
    const isSkills = kind === 'skills';
    const selectedCount = isSkills ? draft.skillNames.length : draft.mcpServerIds.length;
    const rows = isSkills ? catalog.skills : catalog.mcpServers;
    return (
      <section
        ref={refs.setFloating}
        style={floatingStyles}
        className={styles.panel}
        aria-label={isSkills ? t('common.skills') : 'MCP'}
        {...getFloatingProps()}
      >
        <header className={styles.header}>
          <div>
            <h3>{isSkills ? t('common.skills') : 'MCP'}</h3>
            <p>{applyNote}</p>
          </div>
          <Button
            type='text'
            shape='circle'
            size='mini'
            aria-label={t('common.close')}
            icon={<CloseSmall theme='outline' size={16} fill='currentColor' />}
            onClick={() => setOpen(undefined)}
          />
        </header>
        <div className={styles.list} data-testid={`session-${kind}-list`}>
          {loading ? (
            <div className={styles.feedback}><Spin size={20} /></div>
          ) : loadFailed ? (
            <div className={styles.feedback}>
              <span>{t('conversation.capabilityPicker.loadFailed', { defaultValue: '加载失败' })}</span>
              <Button type='text' size='mini' onClick={onRetry}>{t('common.retry')}</Button>
            </div>
          ) : rows.length === 0 ? (
            <div className={styles.feedback}>
              {t('conversation.capabilityPicker.empty', { defaultValue: '暂无可用项目' })}
            </div>
          ) : isSkills ? (
            catalog.skills.map((skill) => {
              const display = resolveSkillDisplay(skill, i18n.language);
              const checked = selectedSkills.has(skill.name);
              return (
                <div
                  key={skill.name}
                  className={`${styles.row} ${disabled ? styles.rowDisabled : ''}`}
                  onClick={() => {
                    if (!disabled) toggleSkill(skill, !checked);
                  }}
                >
                  <CapabilityCheckbox checked={checked} disabled={disabled} label={display.name} onChange={(value) => toggleSkill(skill, value)} />
                  <span className={styles.copy}>
                    <strong>{display.name}</strong>
                    <CapabilityDescription>{display.description || skill.name}</CapabilityDescription>
                  </span>
                  <span className={styles.status}>{skill.source === 'builtin'
                    ? t('conversation.capabilityPicker.builtin', { defaultValue: '内置' })
                    : t('conversation.capabilityPicker.installed', { defaultValue: '已安装' })}</span>
                </div>
              );
            })
          ) : (
            catalog.mcpServers.map((server) => {
              const checked = selectedMcp.has(server.mcp_server_id);
              const status = mcpStatus(server);
              const locked = lockedMcpServerIds.has(server.mcp_server_id);
              const rowDisabled = disabled || locked || (!server.enabled && !checked);
              return (
                <div
                  key={server.mcp_server_id}
                  className={`${styles.row} ${rowDisabled ? styles.rowDisabled : ''}`}
                  onClick={() => {
                    if (!rowDisabled) toggleMcp(server, !checked);
                  }}
                >
                  <CapabilityCheckbox
                    checked={checked}
                    label={server.name}
                    disabled={rowDisabled}
                    onChange={(value) => toggleMcp(server, value)}
                  />
                  <span className={styles.copy}>
                    <strong>{server.name}</strong>
                    <CapabilityDescription>{server.description || t('conversation.capabilityPicker.toolCount', {
                      count: server.tools?.length ?? 0,
                      defaultValue: '{{count}} 个工具',
                    })}</CapabilityDescription>
                  </span>
                  <span className={`${styles.status} ${styles[status]}`}>
                    {status === 'available' && <CheckOne theme='outline' size={12} fill='currentColor' />}
                    {t(`conversation.capabilityPicker.${status}` as const, {
                      defaultValue: status === 'available' ? '可用' : status === 'disabled' ? '已停用' : '异常',
                    })}
                  </span>
                </div>
              );
            })
          )}
        </div>
        <footer className={styles.footer}>
          <span>{t('conversation.capabilityPicker.selectedCount', {
            count: selectedCount,
            defaultValue: `已选 ${selectedCount} 项`,
          })}</span>
          <Button
            type='text'
            size='mini'
            onClick={() => {
              setOpen(undefined);
              void navigate(isSkills ? '/skills' : '/mcp');
            }}
          >
            {isSkills
              ? t('conversation.capabilityPicker.manageSkills', { defaultValue: '管理技能' })
              : t('conversation.capabilityPicker.manageMcp', { defaultValue: '管理 MCP' })}
            <Right theme='outline' size={12} fill='currentColor' />
          </Button>
        </footer>
      </section>
    );
  };

  const trigger = (kind: PickerKind) => {
    const isSkills = kind === 'skills';
    const count = isSkills ? draft.skillNames.length : draft.mcpServerIds.length;
    const title = isSkills ? t('common.skills') : 'MCP';
    return (
      <button
        type='button'
        className={`${styles.railButton} ${open === kind ? styles.active : ''}`}
        {...getReferenceProps({
          onClick: (event: React.MouseEvent<HTMLButtonElement>) => {
            refs.setReference(event.currentTarget);
            // Align above the shared input boundary, even as the textarea grows.
            refs.setPositionReference(event.currentTarget.closest<HTMLElement>('[data-composer-surface]') ?? event.currentTarget);
            setOpen(open === kind ? undefined : kind);
          },
        })}
        aria-label={`${title} · ${count}`}
        aria-expanded={open === kind}
        aria-controls={open === kind ? context.floatingId : undefined}
        title={`${title} · ${count}`}
        data-testid={`session-${kind}-trigger`}
      >
        {isSkills
          ? <Puzzle theme='outline' size={18} fill='currentColor' />
          : <Lightning theme='outline' size={18} strokeWidth={2.5} fill='currentColor' />}
        {count > 0 && <span className={styles.badge} aria-hidden='true'>{count > 9 ? '9+' : count}</span>}
      </button>
    );
  };

  return (
    <aside ref={railRef} className={styles.rail} data-composer-tools aria-label={t('conversation.capabilityPicker.ariaLabel', { defaultValue: '会话能力' })}>
      {trigger('skills')}
      {trigger('mcp')}
      {children}
      {capabilityOpen && (
        <FloatingPortal>
          <FloatingFocusManager context={context} modal={false}>
            {panel(capabilityOpen)}
          </FloatingFocusManager>
        </FloatingPortal>
      )}
    </aside>
  );
};

export {
  buildSessionCapabilitySelection,
  defaultSessionCapabilityDraft,
  draftFromSessionCapabilitySelection,
  sessionCapabilitySelectionKey,
} from './model';
export type { SessionCapabilityCatalog, SessionCapabilityDraft } from './model';
export { useSessionCapabilityCatalog } from './useSessionCapabilityCatalog';
export default SessionCapabilityPicker;
