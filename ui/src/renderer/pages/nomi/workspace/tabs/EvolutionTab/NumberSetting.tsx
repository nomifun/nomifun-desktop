/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import React, { useEffect, useState } from 'react';
import { Message } from '@arco-design/web-react';
import type { InputNumberValueChangeReason } from '@arco-design/web-react/es/InputNumber/interface';
import NomiInputNumber from '@/renderer/components/base/NomiInputNumber';

interface Props {
  value: number;
  min: number;
  max: number;
  suffix?: React.ReactNode;
  /** Persist a validated, in-range value. */
  onCommit: (value: number) => Promise<unknown>;
}

/**
 * A numeric setting that saves on commit, not on keystroke: typing "120" would
 * otherwise write 1 → 12 → 120 (three round-trips, two of them nonsense values).
 * The draft follows the input, the stepper commits immediately, and typing
 * commits when the field loses focus. Out-of-range or empty drafts snap back to
 * the stored value.
 */
const NumberSetting: React.FC<Props> = ({ value, min, max, suffix, onCommit }) => {
  const [draft, setDraft] = useState<number | undefined>(value);

  useEffect(() => {
    setDraft(value);
  }, [value]);

  const commit = (next: number | undefined) => {
    if (next == null || !Number.isFinite(next) || next < min || next > max || !Number.isInteger(next)) {
      setDraft(value);
      return;
    }
    if (next === value) return;
    void onCommit(next).catch((e) => {
      setDraft(value);
      Message.error(String(e));
    });
  };

  return (
    <NomiInputNumber
      contentFit
      min={min}
      max={max}
      precision={0}
      value={draft}
      suffix={suffix}
      onChange={(next: number, reason?: InputNumberValueChangeReason) => {
        setDraft(next);
        // Stepper clicks are already a deliberate, in-range value — save now.
        if (reason === 'increase' || reason === 'decrease') commit(next);
      }}
      onBlur={() => commit(draft)}
    />
  );
};

export default NumberSetting;
