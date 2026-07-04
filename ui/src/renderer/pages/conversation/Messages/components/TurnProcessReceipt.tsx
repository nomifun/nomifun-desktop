/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import type { TurnDisclosureProcessState } from '../turnDisclosureModel';
import { Spin } from '@arco-design/web-react';
import { Attention, Brain, CheckOne, Edit, FolderOpen, Right, Terminal } from '@icon-park/react';
import classNames from 'classnames';
import React, { useEffect, useState } from 'react';

export type TurnProcessReceiptIcon = 'tool' | 'file' | 'edit' | 'thinking' | 'permission' | 'status';

export interface TurnProcessReceiptView<T> {
  id: string;
  item: T;
  label: string;
  state: TurnDisclosureProcessState;
  icon: TurnProcessReceiptIcon;
  defaultExpanded: boolean;
}

interface TurnProcessReceiptProps<T> {
  receipt: TurnProcessReceiptView<T>;
  highlighted?: boolean;
  renderProcessItem: (item: T) => React.ReactNode;
}

const sanitizeDomId = (value: string): string => value.replace(/[^A-Za-z0-9_-]/g, '_');

const ReceiptIcon: React.FC<{
  icon: TurnProcessReceiptIcon;
  state: TurnDisclosureProcessState;
}> = ({ icon, state }) => {
  if (state === 'running') return <Spin size={12} />;
  if (state === 'failed' || state === 'canceled') return <Attention theme='outline' size='15' />;
  if (icon === 'file') return <FolderOpen theme='outline' size='15' />;
  if (icon === 'edit') return <Edit theme='outline' size='15' />;
  if (icon === 'thinking') return <Brain theme='outline' size='15' />;
  if (icon === 'permission') return <Attention theme='outline' size='15' />;
  if (icon === 'status') return <CheckOne theme='outline' size='15' />;
  return <Terminal theme='outline' size='15' />;
};

function TurnProcessReceipt<T>({ receipt, highlighted = false, renderProcessItem }: TurnProcessReceiptProps<T>) {
  const [expanded, setExpanded] = useState(receipt.defaultExpanded);

  useEffect(() => {
    setExpanded(receipt.defaultExpanded);
  }, [receipt.defaultExpanded, receipt.id]);

  useEffect(() => {
    if (highlighted) setExpanded(true);
  }, [highlighted]);

  const bodyId = `turn-process-receipt-body-${sanitizeDomId(receipt.id)}`;

  return (
    <div className={classNames('turn-process-receipt', `turn-process-receipt--${receipt.state}`)}>
      <button
        type='button'
        className='turn-process-receipt__header'
        onClick={() => setExpanded((value) => !value)}
        aria-expanded={expanded}
        aria-controls={bodyId}
      >
        <span className='turn-process-receipt__icon'>
          <ReceiptIcon icon={receipt.icon} state={receipt.state} />
        </span>
        <span className='turn-process-receipt__label'>{receipt.label}</span>
        <Right
          theme='outline'
          size='13'
          className={classNames('turn-process-receipt__arrow', expanded && 'turn-process-receipt__arrow--open')}
        />
      </button>
      {expanded && (
        <div id={bodyId} className='turn-process-receipt__body'>
          {renderProcessItem(receipt.item)}
        </div>
      )}
    </div>
  );
}

export default TurnProcessReceipt;
