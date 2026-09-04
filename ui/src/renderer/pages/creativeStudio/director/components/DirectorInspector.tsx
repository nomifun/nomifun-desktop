/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import { CreativeAssetUnavailable } from '../../assets/components/CreativeAssetUnavailable';
import CreativeMediaPreview from '../../assets/components/CreativeMediaPreview';
import {
  Camera,
  Delete,
  Picture,
  PreviewOpen,
  Send,
  Upload,
} from '@icon-park/react';
import {
  Button,
  Checkbox,
  Input,
  InputNumber,
  Select,
  Slider,
  Tooltip,
} from '@arco-design/web-react';
import React from 'react';
import { useTranslation } from 'react-i18next';

import styles from './DirectorWorkbenchShell.module.css';
import type {
  DirectorCameraInspectorValue,
  DirectorCharacterInspectorValue,
  DirectorEnvironmentInspectorValue,
  DirectorInspectorValue,
  DirectorObjectInspectorValue,
  DirectorVector3,
  DirectorWorkbenchShellProps,
} from './types';

const numericValue = (value: number | undefined, fallback: number): number =>
  typeof value === 'number' && Number.isFinite(value) ? value : fallback;

const VectorEditor: React.FC<{
  label: string;
  value: DirectorVector3;
  step?: number;
  disabled: boolean;
  onChange(value: DirectorVector3): void;
}> = ({ label, value, step = 0.1, disabled, onChange }) => (
  <div className={styles.inspectorField} role='group' aria-label={label}>
    <span className={styles.inspectorFieldLabel}>{label}</span>
    <div className={styles.vectorRow}>
      {(['x', 'y', 'z'] as const).map((axis) => (
        <label key={axis} className={styles.axisField}>
          <span>{axis.toUpperCase()}</span>
          <InputNumber
            aria-label={`${label} ${axis.toUpperCase()}`}
            value={value[axis]}
            step={step}
            precision={3}
            disabled={disabled}
            onChange={(next) =>
              onChange({ ...value, [axis]: numericValue(next, value[axis]) })
            }
          />
        </label>
      ))}
    </div>
  </div>
);

const RangeField: React.FC<{
  label: string;
  value: number;
  min: number;
  max: number;
  step: number;
  disabled: boolean;
  onChange(value: number): void;
}> = ({ label, value, min, max, step, disabled, onChange }) => {
  const { t } = useTranslation();
  return (
    <div className={styles.inspectorField}>
      <span className={styles.inspectorFieldLabel}>{label}</span>
      <div className={styles.rangeRow}>
        <Slider
          aria-label={t('creativeStudio.director.inspector.a11y.slider', {
            defaultValue: '{{label}}滑杆',
            label,
          })}
          value={value}
          min={min}
          max={max}
          step={step}
          disabled={disabled}
          onChange={(next) => {
            if (typeof next === 'number') onChange(next);
          }}
        />
        <InputNumber
          aria-label={label}
          value={value}
          min={min}
          max={max}
          step={step}
          precision={2}
          disabled={disabled}
          onChange={(next) => onChange(numericValue(next, value))}
        />
      </div>
    </div>
  );
};

const ColorField: React.FC<{
  label: string;
  value: string;
  disabled: boolean;
  onChange(value: string): void;
}> = ({ label, value, disabled, onChange }) => {
  const { t } = useTranslation();
  return (
    <label className={styles.inspectorField}>
      <span className={styles.inspectorFieldLabel}>{label}</span>
      <span className={styles.colorRow}>
        <input
          className={styles.colorSwatch}
          aria-label={t('creativeStudio.director.inspector.a11y.colorPicker', {
            defaultValue: '{{label}}取色器',
            label,
          })}
          type='color'
          value={value}
          disabled={disabled}
          onChange={(event) => onChange(event.currentTarget.value)}
        />
        <Input
          aria-label={t('creativeStudio.director.inspector.a11y.hexInput', {
            defaultValue: '{{label}} HEX',
            label,
          })}
          value={value}
          disabled={disabled}
          onChange={onChange}
        />
      </span>
    </label>
  );
};

const InspectorFrame: React.FC<{
  title: string;
  kind: DirectorInspectorValue['kind'];
  children: React.ReactNode;
  tabs?: React.ReactNode;
  footer?: React.ReactNode;
}> = ({ title, kind, children, tabs, footer }) => {
  const { t } = useTranslation();
  return (
    <aside
      className={styles.inspector}
      aria-label={t('creativeStudio.director.inspector.a11y.panel', {
        defaultValue: '{{title}}右侧属性面板',
        title,
      })}
      data-director-inspector={kind}
    >
      <header className={styles.inspectorHeader}>
        <h2>{title}</h2>
      </header>
      {tabs}
      <div className={styles.inspectorScroll}>{children}</div>
      {footer}
    </aside>
  );
};

type InspectorProps = Pick<
  DirectorWorkbenchShellProps,
  | 'inspector'
  | 'bodyTypeOptions'
  | 'posePresetOptions'
  | 'disabled'
  | 'captureBusy'
  | 'onInspectorChange'
  | 'onChoosePanorama'
  | 'onRemovePanorama'
  | 'onReimportObjectModel'
  | 'onPosePresetSelect'
  | 'onCameraCapture'
  | 'onCaptureView'
  | 'onCaptureDelete'
  | 'onCaptureSendToCanvas'
  | 'onCaptureClearAll'
  | 'onCaptureSendAll'
>;

const EnvironmentInspector: React.FC<
  Omit<InspectorProps, 'inspector'> & { value: DirectorEnvironmentInspectorValue }
> = ({
  value,
  disabled = false,
  onInspectorChange,
  onChoosePanorama,
  onRemovePanorama,
}) => {
  const { t } = useTranslation();
  const update = (patch: Partial<DirectorEnvironmentInspectorValue>) =>
    onInspectorChange({ ...value, ...patch });

  return (
    <InspectorFrame
      title={t('creativeStudio.director.inspector.environment.title', {
        defaultValue: '3D场景',
      })}
      kind='environment'
    >
      <RangeField
        label={t('creativeStudio.director.inspector.environment.sceneScale', {
          defaultValue: '场景缩放',
        })}
        value={value.sceneScale}
        min={0.1}
        max={5}
        step={0.05}
        disabled={disabled}
        onChange={(sceneScale) => update({ sceneScale })}
      />
      <VectorEditor
        label={t('creativeStudio.director.inspector.environment.position', {
          defaultValue: '场景平移',
        })}
        value={value.position}
        disabled={disabled}
        onChange={(position) => update({ position })}
      />
      <VectorEditor
        label={t('creativeStudio.director.inspector.environment.rotation', {
          defaultValue: '场景旋转',
        })}
        value={value.rotation}
        disabled={disabled}
        onChange={(rotation) => update({ rotation })}
      />

      <section className={styles.inspectorSection}>
        <h3>
          {t('creativeStudio.director.inspector.environment.panoramaBackground', {
            defaultValue: '全景背景',
          })}
        </h3>
        {value.panorama ? (
          <article className={styles.panoramaCard}>
            <div className={styles.panoramaPreview}>
              {value.panorama.availability && value.panorama.availability !== 'available'
                ? <CreativeAssetUnavailable status={value.panorama.availability} /> : <CreativeMediaPreview
                  kind='image'
                  src={value.panorama.thumbnailUrl}
                  alt={t('creativeStudio.director.inspector.a11y.panoramaThumbnail', {
                    defaultValue: '{{name}} 全景图缩略图',
                    name: value.panorama.name,
                  })}
                />}
            </div>
            <span className={styles.panoramaName}>{value.panorama.name}</span>
            <Button
              type='text'
              shape='circle'
              size='mini'
              aria-label={t(
                'creativeStudio.director.inspector.environment.removePanorama',
                {
                  defaultValue: '删除全景图',
                }
              )}
              icon={<Delete />}
              disabled={disabled || !onRemovePanorama}
              onClick={onRemovePanorama}
            />
          </article>
        ) : (
          <Button
            className={styles.panoramaEmpty}
            icon={<Picture />}
            disabled={disabled || !onChoosePanorama}
            onClick={onChoosePanorama}
          >
            {t('creativeStudio.director.inspector.environment.noPanorama', {
              defaultValue: '未连接全景图',
            })}
          </Button>
        )}
      </section>

      <ColorField
        label={t('creativeStudio.director.inspector.environment.skyColor', {
          defaultValue: '天空颜色',
        })}
        value={value.skyColor}
        disabled={disabled}
        onChange={(skyColor) => update({ skyColor })}
      />
      <section className={styles.inspectorSection}>
        <h3>
          {t('creativeStudio.director.inspector.environment.panoramaSphere', {
            defaultValue: '全景球',
          })}
        </h3>
        <RangeField
          label={t('creativeStudio.director.inspector.environment.panoramaYaw', {
            defaultValue: '水平旋转',
          })}
          value={value.panoramaYaw}
          min={-180}
          max={180}
          step={1}
          disabled={disabled}
          onChange={(panoramaYaw) => update({ panoramaYaw })}
        />
        <RangeField
          label={t('creativeStudio.director.inspector.environment.panoramaRadius', {
            defaultValue: '球形半径',
          })}
          value={value.panoramaRadius}
          min={10}
          max={200}
          step={1}
          disabled={disabled}
          onChange={(panoramaRadius) => update({ panoramaRadius })}
        />
      </section>

      <section className={styles.inspectorSection}>
        <h3>
          {t('creativeStudio.director.inspector.environment.toggles.title', {
            defaultValue: '开关项',
          })}
        </h3>
        <div className={styles.toggleGrid}>
          {[
            [
              t('creativeStudio.director.inspector.environment.toggles.labels', {
                defaultValue: '角色标签',
              }),
              'showLabels',
            ],
            [
              t('creativeStudio.director.inspector.environment.toggles.snapToGrid', {
                defaultValue: '网格吸附',
              }),
              'snapToGrid',
            ],
            [
              t('creativeStudio.director.inspector.environment.toggles.ground', {
                defaultValue: '地面',
              }),
              'showGround',
            ],
            [
              t('creativeStudio.director.inspector.environment.toggles.grid', {
                defaultValue: '网格显示',
              }),
              'showGrid',
            ],
          ].map(([label, key]) => {
            const typedKey = key as 'showLabels' | 'snapToGrid' | 'showGround' | 'showGrid';
            return (
              <Checkbox
                key={key}
                checked={value[typedKey]}
                disabled={disabled}
                onChange={(checked) => update({ [typedKey]: checked })}
              >
                {label}
              </Checkbox>
            );
          })}
        </div>
      </section>

      <RangeField
        label={t('creativeStudio.director.inspector.environment.groundHeight', {
          defaultValue: '地面高度',
        })}
        value={value.groundHeight}
        min={-10}
        max={10}
        step={0.1}
        disabled={disabled || !value.showGround}
        onChange={(groundHeight) => update({ groundHeight })}
      />
      <RangeField
        label={t('creativeStudio.director.inspector.environment.groundOpacity', {
          defaultValue: '地面透明度',
        })}
        value={value.groundOpacity}
        min={0}
        max={1}
        step={0.05}
        disabled={disabled || !value.showGround}
        onChange={(groundOpacity) => update({ groundOpacity })}
      />
    </InspectorFrame>
  );
};

const CameraInspector: React.FC<
  Omit<InspectorProps, 'inspector'> & { value: DirectorCameraInspectorValue }
> = ({
  value,
  disabled = false,
  captureBusy = false,
  onInspectorChange,
  onCameraCapture,
  onCaptureView,
  onCaptureDelete,
  onCaptureSendToCanvas,
  onCaptureClearAll,
  onCaptureSendAll,
}) => {
  const { t } = useTranslation();
  const update = (patch: Partial<DirectorCameraInspectorValue>) =>
    onInspectorChange({ ...value, ...patch });
  const capturesOpen = value.tab === 'captures';
  const tabs = (
    <div
      className={styles.inspectorTabs}
      role='tablist'
      aria-label={t('creativeStudio.director.inspector.camera.tabs', {
        defaultValue: '机位面板标签',
      })}
    >
      {(['properties', 'captures'] as const).map((tab) => (
        <button
          key={tab}
          type='button'
          role='tab'
          aria-selected={value.tab === tab}
          disabled={disabled}
          onClick={() => update({ tab })}
        >
          {tab === 'properties'
            ? t('creativeStudio.director.inspector.camera.propertiesTab', {
                defaultValue: '属性',
              })
            : t('creativeStudio.director.inspector.camera.capturesTab', {
                defaultValue: '截图',
              })}
        </button>
      ))}
    </div>
  );

  const footer = capturesOpen ? (
    <footer className={styles.captureFooter}>
      <Button
        disabled={disabled || value.captures.length === 0 || !onCaptureClearAll}
        onClick={onCaptureClearAll}
      >
        {t('creativeStudio.director.inspector.camera.clearAll', {
          defaultValue: '清空全部',
        })}
      </Button>
      <Button
        type='primary'
        disabled={disabled || value.captures.length === 0 || !onCaptureSendAll}
        onClick={onCaptureSendAll}
      >
        {t('creativeStudio.director.inspector.camera.sendAll', {
          defaultValue: '发送到画布',
        })}
      </Button>
    </footer>
  ) : undefined;

  return (
    <InspectorFrame
      title={t('creativeStudio.director.inspector.camera.title', {
        defaultValue: '机位',
      })}
      kind='camera'
      tabs={tabs}
      footer={footer}
    >
      {capturesOpen ? (
        <section className={styles.captureSection} data-director-capture-list>
          <Button
            className={styles.captureCurrent}
            icon={<Camera />}
            loading={captureBusy}
            disabled={disabled || !onCameraCapture}
            onClick={onCameraCapture}
          >
            {t('creativeStudio.director.inspector.camera.captureCurrent', {
              defaultValue: '当前机位截图',
            })}
          </Button>
          {value.captures.length === 0 ? (
            <div className={styles.captureEmpty} role='status'>
              <Camera aria-hidden='true' size={22} strokeWidth={1.7} />
              <span>
                {t('creativeStudio.director.inspector.camera.empty', {
                  defaultValue: '暂无摄像机截图',
                })}
              </span>
              <small>
                {t('creativeStudio.director.inspector.camera.emptyHint', {
                  defaultValue: '从当前机位生成真实预览后会显示在这里。',
                })}
              </small>
            </div>
          ) : (
            <div className={styles.captureGrid}>
              {value.captures.map((capture) => (
                <article key={capture.id} className={styles.captureCard}>
                  <div className={styles.capturePreview}>
                    {capture.availability && capture.availability !== 'available'
                      ? <CreativeAssetUnavailable status={capture.availability} /> : <CreativeMediaPreview
                        kind='image'
                        src={capture.imageUrl}
                        posterSrc={capture.thumbnailUrl}
                        alt={t('creativeStudio.director.inspector.a11y.thumbnail', {
                          defaultValue: '{{name}} 缩略图',
                          name: capture.name,
                        })}
                      />}
                  </div>
                  <strong title={capture.name}>{capture.name}</strong>
                  <div className={styles.captureActions}>
                    <Tooltip
                      content={t(
                        'creativeStudio.director.inspector.camera.viewCapture',
                        {
                          defaultValue: '查看截图',
                        }
                      )}
                    >
                      <Button
                        type='text'
                        size='mini'
                        shape='circle'
                        aria-label={t(
                          'creativeStudio.director.inspector.camera.viewCaptureNamed',
                          {
                            defaultValue: '查看截图 {{name}}',
                            name: capture.name,
                          }
                        )}
                        icon={<PreviewOpen />}
                        disabled={disabled || !onCaptureView || Boolean(capture.availability && capture.availability !== 'available')}
                        onClick={() => onCaptureView?.(capture)}
                      />
                    </Tooltip>
                    <Tooltip
                      content={t(
                        'creativeStudio.director.inspector.camera.sendCapture',
                        {
                          defaultValue: '发送到画布',
                        }
                      )}
                    >
                      <Button
                        type='text'
                        size='mini'
                        shape='circle'
                        aria-label={t(
                          'creativeStudio.director.inspector.camera.sendCaptureNamed',
                          {
                            defaultValue: '发送到画布 {{name}}',
                            name: capture.name,
                          }
                        )}
                        icon={<Send />}
                        disabled={disabled || !onCaptureSendToCanvas || Boolean(capture.availability && capture.availability !== 'available')}
                        onClick={() => onCaptureSendToCanvas?.(capture)}
                      />
                    </Tooltip>
                    <Tooltip
                      content={t(
                        'creativeStudio.director.inspector.camera.deleteCapture',
                        {
                          defaultValue: '删除截图',
                        }
                      )}
                    >
                      <Button
                        type='text'
                        size='mini'
                        shape='circle'
                        aria-label={t(
                          'creativeStudio.director.inspector.camera.deleteCaptureNamed',
                          {
                            defaultValue: '删除截图 {{name}}',
                            name: capture.name,
                          }
                        )}
                        icon={<Delete />}
                        disabled={disabled || !onCaptureDelete}
                        onClick={() => onCaptureDelete?.(capture.id)}
                      />
                    </Tooltip>
                  </div>
                </article>
              ))}
            </div>
          )}
        </section>
      ) : (
        <>
          <label className={styles.inspectorField}>
            <span className={styles.inspectorFieldLabel}>
              {t('creativeStudio.director.inspector.camera.name', {
                defaultValue: '机位名称',
              })}
            </span>
            <Input
              value={value.name}
              disabled={disabled}
              onChange={(name) => update({ name })}
            />
          </label>
          {value.targetLabel ? (
            <label className={styles.inspectorField}>
              <span className={styles.inspectorFieldLabel}>
                {t('creativeStudio.director.inspector.camera.target', {
                  defaultValue: '注视目标',
                })}
              </span>
              <Input value={value.targetLabel} readOnly />
            </label>
          ) : null}
          <VectorEditor
            label={t('creativeStudio.director.inspector.camera.position', {
              defaultValue: '机位位置',
            })}
            value={value.position}
            disabled={disabled}
            onChange={(position) => update({ position })}
          />
          <VectorEditor
            label={t('creativeStudio.director.inspector.camera.rotation', {
              defaultValue: '机位旋转',
            })}
            value={value.rotation}
            disabled={disabled}
            onChange={(rotation) => update({ rotation })}
          />
          <RangeField
            label={t('creativeStudio.director.inspector.camera.fov', {
              defaultValue: '机位 FOV',
            })}
            value={value.fov}
            min={10}
            max={120}
            step={1}
            disabled={disabled}
            onChange={(fov) => update({ fov })}
          />
        </>
      )}
    </InspectorFrame>
  );
};

const CharacterInspector: React.FC<
  Omit<InspectorProps, 'inspector'> & { value: DirectorCharacterInspectorValue }
> = ({
  value,
  bodyTypeOptions,
  posePresetOptions,
  disabled = false,
  onInspectorChange,
  onPosePresetSelect,
}) => {
  const { t } = useTranslation();
  const update = (patch: Partial<DirectorCharacterInspectorValue>) =>
    onInspectorChange({ ...value, ...patch });

  return (
    <InspectorFrame
      title={t('creativeStudio.director.inspector.character.title', {
        defaultValue: '角色',
      })}
      kind='character'
    >
      <label className={styles.inspectorField}>
        <span className={styles.inspectorFieldLabel}>
          {t('creativeStudio.director.inspector.character.name', {
            defaultValue: '角色名称',
          })}
        </span>
        <Input value={value.name} disabled={disabled} onChange={(name) => update({ name })} />
      </label>
      <label className={styles.inspectorField}>
        <span className={styles.inspectorFieldLabel}>
          {t('creativeStudio.director.inspector.character.bodyType', {
            defaultValue: '角色体型',
          })}
        </span>
        <Select
          value={value.bodyType}
          options={[...bodyTypeOptions]}
          disabled={disabled}
          onChange={(bodyType) => {
            if (typeof bodyType === 'string') update({ bodyType });
          }}
        />
      </label>
      <VectorEditor
        label={t('creativeStudio.director.inspector.character.position', {
          defaultValue: '角色位置',
        })}
        value={value.position}
        disabled={disabled}
        onChange={(position) => update({ position })}
      />
      <VectorEditor
        label={t('creativeStudio.director.inspector.character.rotation', {
          defaultValue: '角色旋转',
        })}
        value={value.rotation}
        disabled={disabled}
        onChange={(rotation) => update({ rotation })}
      />
      <RangeField
        label={t('creativeStudio.director.inspector.character.scale', {
          defaultValue: '统一缩放',
        })}
        value={value.scale}
        min={0.1}
        max={5}
        step={0.05}
        disabled={disabled}
        onChange={(scale) => update({ scale })}
      />
      <ColorField
        label={t('creativeStudio.director.inspector.character.color', {
          defaultValue: '角色颜色',
        })}
        value={value.color}
        disabled={disabled}
        onChange={(color) => update({ color })}
      />
      <section className={styles.inspectorSection}>
        <h3>
          {t('creativeStudio.director.inspector.character.posePresets', {
            defaultValue: '姿势预设',
          })}
        </h3>
        <div
          className={styles.posePresetGrid}
          role='listbox'
          aria-label={t('creativeStudio.director.inspector.character.posePresets', {
            defaultValue: '姿势预设',
          })}
        >
          {posePresetOptions.map((option) => (
            <Button
              key={option.value}
              aria-pressed={value.posePresetId === option.value}
              disabled={disabled || option.disabled || !onPosePresetSelect}
              onClick={() => onPosePresetSelect?.(option.value)}
            >
              {option.label}
            </Button>
          ))}
        </div>
      </section>
    </InspectorFrame>
  );
};

const ObjectInspector: React.FC<
  Omit<InspectorProps, 'inspector'> & { value: DirectorObjectInspectorValue }
> = ({ value, disabled = false, onInspectorChange, onReimportObjectModel }) => {
  const { t } = useTranslation();
  const update = (patch: Partial<DirectorObjectInspectorValue>) =>
    onInspectorChange({ ...value, ...patch });

  return (
    <InspectorFrame
      title={t('creativeStudio.director.inspector.object.title', {
        defaultValue: '物体',
      })}
      kind='object'
    >
      <label className={styles.inspectorField}>
        <span className={styles.inspectorFieldLabel}>
          {t('creativeStudio.director.inspector.object.name', {
            defaultValue: '模型名称',
          })}
        </span>
        <Input value={value.name} disabled={disabled} onChange={(name) => update({ name })} />
      </label>
      {value.modelLabel ? (
        <label className={styles.inspectorField}>
          <span className={styles.inspectorFieldLabel}>
            {t('creativeStudio.director.inspector.object.source', {
              defaultValue: '模型来源',
            })}
          </span>
          <Input value={value.modelLabel} readOnly />
        </label>
      ) : null}
      {value.localAssetMissing ? (
        <section className={styles.missingModel} role='alert'>
          <p>
            {t('creativeStudio.director.inspector.object.missingAsset', {
              defaultValue:
                '模型文件只保存在原导入设备，重新导入后会继承当前模型数据。',
            })}
          </p>
          <Button
            icon={<Upload />}
            disabled={disabled || !onReimportObjectModel}
            onClick={onReimportObjectModel}
          >
            {t('creativeStudio.director.inspector.object.reimport', {
              defaultValue: '重新导入模型',
            })}
          </Button>
        </section>
      ) : null}
      <VectorEditor
        label={t('creativeStudio.director.inspector.object.position', {
          defaultValue: '模型位置',
        })}
        value={value.position}
        disabled={disabled}
        onChange={(position) => update({ position })}
      />
      <VectorEditor
        label={t('creativeStudio.director.inspector.object.rotation', {
          defaultValue: '模型旋转',
        })}
        value={value.rotation}
        disabled={disabled}
        onChange={(rotation) => update({ rotation })}
      />
      <RangeField
        label={t('creativeStudio.director.inspector.object.scale', {
          defaultValue: '统一缩放',
        })}
        value={value.scale}
        min={0.1}
        max={5}
        step={0.05}
        disabled={disabled}
        onChange={(scale) => update({ scale })}
      />
      <ColorField
        label={t('creativeStudio.director.inspector.object.color', {
          defaultValue: '模型颜色',
        })}
        value={value.color}
        disabled={disabled}
        onChange={(color) => update({ color })}
      />
    </InspectorFrame>
  );
};

const DirectorInspector: React.FC<InspectorProps> = (props) => {
  if (props.inspector.kind === 'environment') {
    return <EnvironmentInspector {...props} value={props.inspector} />;
  }
  if (props.inspector.kind === 'camera') {
    return <CameraInspector {...props} value={props.inspector} />;
  }
  if (props.inspector.kind === 'character') {
    return <CharacterInspector {...props} value={props.inspector} />;
  }
  return <ObjectInspector {...props} value={props.inspector} />;
};

export default DirectorInspector;
