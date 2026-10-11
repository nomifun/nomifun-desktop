/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { Pic, Plus } from '@icon-park/react';
import styles from './CreativeTemplateWorkspacePage.module.css';

/** StateEmpty's four template slots, adapted from codevisual-share with theme
 * tokens and no looping demo animation. The real create action lives below. */
export default function TemplateWorkspaceEmptyIllustration() {
  return (
    <div className={styles.emptyIllustration} aria-hidden='true'>
      <div className={styles.emptySlots}>
        {[0, 1, 2, 3].map((slot) => (
          <div key={slot} className={styles.emptySlot} data-filled={slot === 0}>
            <span className={styles.emptySlotIcon}><Pic theme='outline' size={13} fill='currentColor' /></span>
            <div className={styles.emptySlotLines}><span /><span /></div>
          </div>
        ))}
      </div>
      <span className={styles.emptyPlus}><Plus theme='outline' size={18} fill='currentColor' /></span>
    </div>
  );
}
