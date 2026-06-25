/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 * Based on AionUi (https://github.com/iOfficeAI/AionUi)
 */

import React from 'react';
import styles from '../index.module.css';

/**
 * Skeleton placeholder for the AgentPillBar while agents are loading.
 * Mimics the pill bar container with 5 circular shimmer elements.
 */
export const AgentPillBarSkeleton: React.FC = () => {
  return (
    <div className='w-full flex justify-center'>
      <div
        className='inline-flex items-center bg-fill-2'
        style={{
          marginBottom: 16,
          padding: '4px',
          borderRadius: '30px',
          gap: 12,
        }}
      >
        {/* First pill is wider to mimic the selected state */}
        <div className={styles.skeleton} style={{ width: 48, height: 28, borderRadius: 20 }} />
        {[28, 28, 28, 28].map((size, i) => (
          <div key={i} className={styles.skeleton} style={{ width: size, height: size, borderRadius: '50%' }} />
        ))}
      </div>
    </div>
  );
};
