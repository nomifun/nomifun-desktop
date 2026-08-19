/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { LinkOne, MagicWand, Platte } from '@icon-park/react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import styles from './CreativeStudioHomePage.module.css';

const CreativeStudioHomePage: React.FC = () => {
  const { t } = useTranslation();
  const pillars = [
    {
      key: 'canvas',
      icon: <Platte theme='outline' size={21} fill='currentColor' />,
      title: t('creativeStudio.home.canvas.title'),
      description: t('creativeStudio.home.canvas.description'),
    },
    {
      key: 'nodes',
      icon: <LinkOne theme='outline' size={21} fill='currentColor' />,
      title: t('creativeStudio.home.nodes.title'),
      description: t('creativeStudio.home.nodes.description'),
    },
    {
      key: 'models',
      icon: <MagicWand theme='outline' size={21} fill='currentColor' />,
      title: t('creativeStudio.home.models.title'),
      description: t('creativeStudio.home.models.description'),
    },
  ];

  return (
    <section className={styles.page} data-creative-studio-home>
      <div className={styles.glow} aria-hidden='true' />
      <div className={styles.grid} aria-hidden='true' />
      <div className={styles.hero}>
        <div className={styles.eyebrow}>{t('creativeStudio.home.eyebrow')}</div>
        <h1 className={styles.title}>{t('creativeStudio.home.headline')}</h1>
        <p className={styles.description}>{t('creativeStudio.home.description')}</p>
        <div className={styles.pillars}>
          {pillars.map((pillar) => (
            <article key={pillar.key} className={styles.pillar}>
              <div className={styles.icon}>{pillar.icon}</div>
              <div>
                <h2 className={styles.pillarTitle}>{pillar.title}</h2>
                <p className={styles.pillarDescription}>{pillar.description}</p>
              </div>
            </article>
          ))}
        </div>
      </div>
    </section>
  );
};

export default CreativeStudioHomePage;
