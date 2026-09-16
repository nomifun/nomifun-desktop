import { Button, type ButtonProps } from '@arco-design/web-react';
import type { PropsWithChildren } from 'react';
import styles from './AgentSettingsPage.module.css';

export function AgentEditorActionBar({ children }: PropsWithChildren) {
  return <footer className={styles.actionBar}>{children}</footer>;
}

export function AgentEditorActionButton({ className, ...props }: Omit<ButtonProps, 'type' | 'size'>) {
  return <Button {...props} type='secondary' size='small' className={[styles.editorActionButton, className].filter(Boolean).join(' ')} />;
}
