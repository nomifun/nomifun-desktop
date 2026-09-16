import { useState, type ComponentProps, type HTMLAttributes, type ReactNode, type Ref } from 'react';
import { Button, Input } from '@arco-design/web-react';
import { ArrowUp } from '@icon-park/react';
import { useTranslation } from 'react-i18next';
import { useInputFocusRing } from '@/renderer/hooks/chat/useInputFocusRing';
import { useCompositionInput } from '@/renderer/hooks/chat/useCompositionInput';
import UploadProgressBar from '@/renderer/components/media/UploadProgressBar';
import ResponsiveComposerRow from './ResponsiveComposerRow';
import { SessionCapabilityComposerLayout } from './SessionCapabilityPicker/ComposerLayout';
import './SendBox/sendbox.css';

type ComposerInputProps = Omit<ComponentProps<typeof Input.TextArea>, 'className' | 'style' | 'autoSize'> & { 'data-testid'?: string };

export type ComposerProps = {
  inputProps: ComposerInputProps;
  surfaceRef?: Ref<HTMLDivElement>;
  singleLine?: boolean;
  isFileDragging?: boolean;
  dragHandlers?: HTMLAttributes<HTMLDivElement>;
  overlayOpen?: boolean;
  overlays?: ReactNode;
  header?: ReactNode;
  beforeInput?: ReactNode;
  inputOverlay?: ReactNode;
  highlightInput?: boolean;
  attachments?: ReactNode;
  tools?: ReactNode;
  creationTools?: ReactNode;
  rightTools?: ReactNode;
  actions?: ReactNode;
  sideTools?: ReactNode;
  topRightTools?: ReactNode;
  footer?: ReactNode;
};

/** The single composer renderer. Home and active conversations supply behavior,
 * not competing shells, editors, toolbar layouts, or visual variants. */
export default function Composer({ inputProps, surfaceRef, singleLine = false, isFileDragging = false,
  dragHandlers, overlayOpen = false, overlays, header, beforeInput, inputOverlay, highlightInput = false,
  attachments, tools, creationTools, rightTools, actions, sideTools, topRightTools, footer,
}: ComposerProps) {
  const [focused, setFocused] = useState(false);
  const { activeBorderColor, inactiveBorderColor, activeShadow } = useInputFocusRing();
  const { compositionHandlers, isImeActive } = useCompositionInput();
  return <>
    <div ref={surfaceRef} data-composer-surface
      className={`sendbox-panel relative p-16px border-3 b bg-dialog-fill-0 b-solid rd-20px flex flex-col ${sideTools ? 'sendbox-panel--side-tools' : ''} ${overlayOpen ? 'overflow-visible' : 'overflow-hidden'} ${isFileDragging ? 'b-dashed sendbox-panel--dragging' : ''}`}
      style={{
        transition: 'box-shadow 0.25s ease, border-color 0.25s ease',
        padding: sideTools ? 0 : undefined,
        borderRadius: sideTools ? 22 : undefined,
        borderWidth: '1px',
        borderColor: isFileDragging ? 'rgb(var(--primary-3))' : focused ? activeBorderColor : inactiveBorderColor,
        boxShadow: focused ? activeShadow : 'none',
        ...(isFileDragging ? { backgroundColor: 'var(--color-primary-light-1)' } : {}),
      }} {...dragHandlers}>
      <SessionCapabilityComposerLayout picker={sideTools}>
        {overlays}
        {topRightTools && <div className='sendbox-internal-status-row mb-8px flex w-full flex-wrap items-start gap-8px' data-testid='sendbox-internal-status-row'>
          <div className='ml-auto flex h-28px flex-shrink-0 items-center' data-testid='sendbox-internal-context-tools'>{topRightTools}</div>
        </div>}
        {header}
        {beforeInput}
        <UploadProgressBar source='sendbox' />
        <div className={singleLine ? 'flex items-center gap-2 w-full min-w-0 overflow-hidden' : 'w-full overflow-hidden'}>
          {singleLine && <div className='flex-shrink-0 sendbox-tools'>{tools}</div>}
          <div className={`sendbox-highlight-container ${singleLine ? 'sendbox-highlight-container--single' : ''}`}
            style={{ width: singleLine ? 'auto' : '100%', flex: singleLine ? 1 : 'none', minWidth: 0, maxWidth: '100%', marginBottom: singleLine ? 0 : sideTools ? 6 : 8, minHeight: singleLine ? 20 : 40 }}>
            {inputOverlay}
            <Input.TextArea {...inputProps}
              spellCheck={false}
              className={`${highlightInput ? 'sendbox-highlight-textarea ' : ''}pl-0 pr-0 !b-none focus:shadow-none m-0 !bg-transparent !focus:bg-transparent !hover:bg-transparent lh-[20px] !resize-none text-14px`}
              style={{ width: '100%', flex: singleLine ? 1 : 'none', minWidth: 0, maxWidth: '100%', margin: 0, height: singleLine ? 20 : 'auto', minHeight: singleLine ? 20 : 40, overflowY: singleLine ? 'hidden' : 'auto', overflowX: 'hidden', whiteSpace: singleLine ? 'nowrap' : 'pre-wrap', textOverflow: singleLine ? 'ellipsis' : 'clip', wordBreak: singleLine ? 'normal' : 'break-word', overflowWrap: 'break-word' }}
              autoSize={singleLine ? false : { minRows: 2, maxRows: 10 }}
              onFocus={event => { setFocused(true); inputProps.onFocus?.(event); }}
              onBlur={event => { setFocused(false); inputProps.onBlur?.(event); }}
              onCompositionStartCapture={event => { compositionHandlers.onCompositionStartCapture(); inputProps.onCompositionStartCapture?.(event); }}
              onCompositionEndCapture={event => { compositionHandlers.onCompositionEndCapture(); inputProps.onCompositionEndCapture?.(event); }}
              onKeyDown={event => { if (!isImeActive(event)) inputProps.onKeyDown?.(event); }}
            />
          </div>
          {singleLine && <div className='flex items-center gap-2'>{actions}</div>}
        </div>
        {attachments}
        {!singleLine && <ResponsiveComposerRow className='sendbox-bottom-row flex items-center justify-between gap-2 w-full'>
          <div className='sendbox-tools'>{tools}</div>
          {creationTools}
          <div data-composer-group className='sendbox-actions flex items-center gap-2' style={{ marginLeft: 'auto', maxWidth: '100%' }}>
            {rightTools}
            {actions}
          </div>
        </ResponsiveComposerRow>}
      </SessionCapabilityComposerLayout>
    </div>
    {footer}
  </>;
}

export function ComposerSendButton({ disabled, loading, onClick, icon, title, testId = 'sendbox-send-btn' }: {
  disabled?: boolean;
  loading?: boolean;
  onClick: () => void;
  icon?: ReactNode;
  title?: string;
  testId?: string;
}) {
  const { t } = useTranslation();
  return <Button shape='circle' type='primary' disabled={disabled} loading={loading}
    className='send-button-custom' title={title} aria-label={title ?? t('common.send')}
    icon={icon ?? <ArrowUp theme='filled' size='14' fill='currentColor' strokeWidth={5} />}
    onClick={onClick} data-testid={testId} data-composer-action='send' />;
}
