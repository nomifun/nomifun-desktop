/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  Camera,
  Cube,
  Lock,
  People,
  Peoples,
  PreviewClose,
  PreviewOpen,
  Search,
  Unlock,
} from '@icon-park/react';
import { Button, Input, Tooltip } from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import styles from './DirectorWorkbenchShell.module.css';
import type { DirectorSceneObject, DirectorWorkbenchShellProps } from './types';

const SceneObjectIcon: React.FC<{ object: DirectorSceneObject }> = ({ object }) => {
  const iconProps = { size: 16, strokeWidth: 1.8, fill: 'currentColor' };
  if (object.kind === 'camera') return <Camera {...iconProps} />;
  if (object.kind === 'character') return <People {...iconProps} />;
  if (object.kind === 'crowd') return <Peoples {...iconProps} />;
  return <Cube {...iconProps} />;
};

type DirectorSceneSidebarProps = Pick<
  DirectorWorkbenchShellProps,
  | 'sceneQuery'
  | 'sceneGroups'
  | 'disabled'
  | 'onSceneQueryChange'
  | 'onSceneObjectSelect'
  | 'onSceneObjectVisibilityChange'
  | 'onSceneObjectLockChange'
>;

const DirectorSceneSidebar: React.FC<DirectorSceneSidebarProps> = ({
  sceneQuery,
  sceneGroups,
  disabled = false,
  onSceneQueryChange,
  onSceneObjectSelect,
  onSceneObjectVisibilityChange,
  onSceneObjectLockChange,
}) => {
  const { t } = useTranslation();
  const visibleGroups = sceneGroups
    .map((group) => ({
      ...group,
      objects: group.objects.filter((object) =>
        object.name.toLocaleLowerCase().includes(sceneQuery.trim().toLocaleLowerCase())
      ),
    }))
    .filter((group) => group.objects.length > 0);

  return (
    <aside
      className={styles.sceneSidebar}
      aria-label={t('creativeStudio.director.scene.title', {
        defaultValue: '场景对象',
      })}
      data-director-scene-sidebar
    >
      <label className={styles.searchField}>
        <Search aria-hidden='true' size={15} strokeWidth={1.8} />
        <Input
          aria-label={t('creativeStudio.director.scene.search.label', {
            defaultValue: '搜索场景内容',
          })}
          value={sceneQuery}
          placeholder={t('creativeStudio.director.scene.search.placeholder', {
            defaultValue: '请输入搜索内容',
          })}
          disabled={disabled}
          onChange={onSceneQueryChange}
        />
      </label>

      {visibleGroups.length === 0 ? (
        <div className={styles.sceneEmpty} role='status'>
          <Search aria-hidden='true' size={22} strokeWidth={1.7} />
          <span>
            {sceneQuery.trim()
              ? t('creativeStudio.director.scene.search.empty', {
                  defaultValue: '未搜索到内容',
                })
              : t('creativeStudio.director.scene.empty', {
                  defaultValue: '场景中还没有对象',
                })}
          </span>
        </div>
      ) : (
        <div
          className={styles.sceneGroups}
          role='tree'
          aria-label={t('creativeStudio.director.scene.list.label', {
            defaultValue: '场景对象列表',
          })}
        >
          {visibleGroups.map((group) => (
            <section key={group.id} className={styles.sceneGroup} role='group' aria-label={group.label}>
              <h2>{group.label}</h2>
              <ul className={styles.sceneList}>
                {group.objects.map((object) => {
                  const rowClassName = [
                    styles.objectRow,
                    object.selected ? styles.objectRowSelected : '',
                    object.missingLocalAsset ? styles.objectRowMissing : '',
                  ]
                    .filter(Boolean)
                    .join(' ');

                  return (
                    <li key={object.id}>
                      <div
                        className={rowClassName}
                        role='treeitem'
                        aria-selected={object.selected || false}
                        data-scene-object-kind={object.kind}
                      >
                        <button
                          type='button'
                          className={styles.objectSelect}
                          disabled={disabled}
                          onClick={() => onSceneObjectSelect(object.id)}
                        >
                          <span className={styles.objectKindIcon} aria-hidden='true'>
                            <SceneObjectIcon object={object} />
                          </span>
                          <span title={object.name}>{object.name}</span>
                        </button>

                        <Tooltip
                          content={
                            object.visible
                              ? t('creativeStudio.director.scene.object.hide', {
                                  defaultValue: '在视口中隐藏',
                                })
                              : t('creativeStudio.director.scene.object.show', {
                                  defaultValue: '在视口中显示',
                                })
                          }
                        >
                          <Button
                            type='text'
                            size='mini'
                            shape='circle'
                            className={styles.objectFlag}
                            aria-label={t(
                              'creativeStudio.director.scene.object.visibility',
                              {
                                defaultValue: '{{name}} 可见性',
                                name: object.name,
                              }
                            )}
                            aria-pressed={object.visible}
                            disabled={disabled}
                            icon={object.visible ? <PreviewOpen /> : <PreviewClose />}
                            onClick={() =>
                              onSceneObjectVisibilityChange(object.id, !object.visible)
                            }
                          />
                        </Tooltip>

                        <Tooltip
                          content={
                            object.locked
                              ? t('creativeStudio.director.scene.object.unlock', {
                                  defaultValue: '解锁对象',
                                })
                              : t('creativeStudio.director.scene.object.lock', {
                                  defaultValue: '锁定对象',
                                })
                          }
                        >
                          <Button
                            type='text'
                            size='mini'
                            shape='circle'
                            className={styles.objectFlag}
                            aria-label={t(
                              'creativeStudio.director.scene.object.lockState',
                              {
                                defaultValue: '{{name}} 锁定',
                                name: object.name,
                              }
                            )}
                            aria-pressed={object.locked}
                            disabled={disabled}
                            icon={object.locked ? <Lock /> : <Unlock />}
                            onClick={() => onSceneObjectLockChange(object.id, !object.locked)}
                          />
                        </Tooltip>
                      </div>
                    </li>
                  );
                })}
              </ul>
            </section>
          ))}
        </div>
      )}
    </aside>
  );
};

export default DirectorSceneSidebar;
