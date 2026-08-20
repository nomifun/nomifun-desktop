/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

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
}> = ({ label, value, min, max, step, disabled, onChange }) => (
  <div className={styles.inspectorField}>
    <span className={styles.inspectorFieldLabel}>{label}</span>
    <div className={styles.rangeRow}>
      <Slider
        aria-label={`${label}滑杆`}
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

const ColorField: React.FC<{
  label: string;
  value: string;
  disabled: boolean;
  onChange(value: string): void;
}> = ({ label, value, disabled, onChange }) => (
  <label className={styles.inspectorField}>
    <span className={styles.inspectorFieldLabel}>{label}</span>
    <span className={styles.colorRow}>
      <input
        className={styles.colorSwatch}
        aria-label={`${label}取色器`}
        type='color'
        value={value}
        disabled={disabled}
        onChange={(event) => onChange(event.currentTarget.value)}
      />
      <Input
        aria-label={`${label} HEX`}
        value={value}
        disabled={disabled}
        onChange={onChange}
      />
    </span>
  </label>
);

const InspectorFrame: React.FC<{
  title: string;
  kind: DirectorInspectorValue['kind'];
  children: React.ReactNode;
  tabs?: React.ReactNode;
  footer?: React.ReactNode;
}> = ({ title, kind, children, tabs, footer }) => (
  <aside
    className={styles.inspector}
    aria-label={`${title}右侧属性面板`}
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
  const update = (patch: Partial<DirectorEnvironmentInspectorValue>) =>
    onInspectorChange({ ...value, ...patch });

  return (
    <InspectorFrame title='3D场景' kind='environment'>
      <RangeField
        label='场景缩放'
        value={value.sceneScale}
        min={0.1}
        max={5}
        step={0.05}
        disabled={disabled}
        onChange={(sceneScale) => update({ sceneScale })}
      />
      <VectorEditor
        label='场景平移'
        value={value.position}
        disabled={disabled}
        onChange={(position) => update({ position })}
      />
      <VectorEditor
        label='场景旋转'
        value={value.rotation}
        disabled={disabled}
        onChange={(rotation) => update({ rotation })}
      />

      <section className={styles.inspectorSection}>
        <h3>全景背景</h3>
        {value.panorama ? (
          <article className={styles.panoramaCard}>
            <img src={value.panorama.thumbnailUrl} alt={`${value.panorama.name} 全景图缩略图`} />
            <span>{value.panorama.name}</span>
            <Button
              type='text'
              shape='circle'
              size='mini'
              aria-label='删除全景图'
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
            未连接全景图
          </Button>
        )}
      </section>

      <ColorField
        label='天空颜色'
        value={value.skyColor}
        disabled={disabled}
        onChange={(skyColor) => update({ skyColor })}
      />
      <section className={styles.inspectorSection}>
        <h3>全景球</h3>
        <RangeField
          label='水平旋转'
          value={value.panoramaYaw}
          min={-180}
          max={180}
          step={1}
          disabled={disabled}
          onChange={(panoramaYaw) => update({ panoramaYaw })}
        />
        <RangeField
          label='球形半径'
          value={value.panoramaRadius}
          min={10}
          max={200}
          step={1}
          disabled={disabled}
          onChange={(panoramaRadius) => update({ panoramaRadius })}
        />
      </section>

      <section className={styles.inspectorSection}>
        <h3>开关项</h3>
        <div className={styles.toggleGrid}>
          {[
            ['角色标签', 'showLabels'],
            ['网格吸附', 'snapToGrid'],
            ['地面', 'showGround'],
            ['网格显示', 'showGrid'],
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
        label='地面高度'
        value={value.groundHeight}
        min={-10}
        max={10}
        step={0.1}
        disabled={disabled || !value.showGround}
        onChange={(groundHeight) => update({ groundHeight })}
      />
      <RangeField
        label='地面透明度'
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
  const update = (patch: Partial<DirectorCameraInspectorValue>) =>
    onInspectorChange({ ...value, ...patch });
  const capturesOpen = value.tab === 'captures';
  const tabs = (
    <div className={styles.inspectorTabs} role='tablist' aria-label='机位面板标签'>
      {(['properties', 'captures'] as const).map((tab) => (
        <button
          key={tab}
          type='button'
          role='tab'
          aria-selected={value.tab === tab}
          disabled={disabled}
          onClick={() => update({ tab })}
        >
          {tab === 'properties' ? '属性' : '截图'}
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
        清空全部
      </Button>
      <Button
        type='primary'
        disabled={disabled || value.captures.length === 0 || !onCaptureSendAll}
        onClick={onCaptureSendAll}
      >
        发送全部
      </Button>
    </footer>
  ) : undefined;

  return (
    <InspectorFrame title='机位' kind='camera' tabs={tabs} footer={footer}>
      {capturesOpen ? (
        <section className={styles.captureSection} data-director-capture-list>
          <Button
            className={styles.captureCurrent}
            icon={<Camera />}
            loading={captureBusy}
            disabled={disabled || !onCameraCapture}
            onClick={onCameraCapture}
          >
            当前机位截图
          </Button>
          {value.captures.length === 0 ? (
            <div className={styles.captureEmpty} role='status'>
              <Camera aria-hidden='true' size={22} strokeWidth={1.7} />
              <span>暂无摄像机截图</span>
              <small>从当前机位生成真实预览后会显示在这里。</small>
            </div>
          ) : (
            <div className={styles.captureGrid}>
              {value.captures.map((capture) => (
                <article key={capture.id} className={styles.captureCard}>
                  <img src={capture.thumbnailUrl} alt={`${capture.name} 缩略图`} />
                  <strong title={capture.name}>{capture.name}</strong>
                  <div className={styles.captureActions}>
                    <Tooltip content='查看截图'>
                      <Button
                        type='text'
                        size='mini'
                        shape='circle'
                        aria-label={`查看截图 ${capture.name}`}
                        icon={<PreviewOpen />}
                        disabled={disabled || !onCaptureView}
                        onClick={() => onCaptureView?.(capture)}
                      />
                    </Tooltip>
                    <Tooltip content='发送到画布'>
                      <Button
                        type='text'
                        size='mini'
                        shape='circle'
                        aria-label={`发送到画布 ${capture.name}`}
                        icon={<Send />}
                        disabled={disabled || !onCaptureSendToCanvas}
                        onClick={() => onCaptureSendToCanvas?.(capture)}
                      />
                    </Tooltip>
                    <Tooltip content='删除截图'>
                      <Button
                        type='text'
                        size='mini'
                        shape='circle'
                        aria-label={`删除截图 ${capture.name}`}
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
            <span className={styles.inspectorFieldLabel}>机位名称</span>
            <Input
              value={value.name}
              disabled={disabled}
              onChange={(name) => update({ name })}
            />
          </label>
          {value.targetLabel ? (
            <label className={styles.inspectorField}>
              <span className={styles.inspectorFieldLabel}>注视目标</span>
              <Input value={value.targetLabel} readOnly />
            </label>
          ) : null}
          <VectorEditor
            label='机位位置'
            value={value.position}
            disabled={disabled}
            onChange={(position) => update({ position })}
          />
          <VectorEditor
            label='机位旋转'
            value={value.rotation}
            disabled={disabled}
            onChange={(rotation) => update({ rotation })}
          />
          <RangeField
            label='机位 FOV'
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
  const update = (patch: Partial<DirectorCharacterInspectorValue>) =>
    onInspectorChange({ ...value, ...patch });

  return (
    <InspectorFrame title='角色' kind='character'>
      <label className={styles.inspectorField}>
        <span className={styles.inspectorFieldLabel}>角色名称</span>
        <Input value={value.name} disabled={disabled} onChange={(name) => update({ name })} />
      </label>
      <label className={styles.inspectorField}>
        <span className={styles.inspectorFieldLabel}>角色体型</span>
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
        label='角色位置'
        value={value.position}
        disabled={disabled}
        onChange={(position) => update({ position })}
      />
      <VectorEditor
        label='角色旋转'
        value={value.rotation}
        disabled={disabled}
        onChange={(rotation) => update({ rotation })}
      />
      <RangeField
        label='统一缩放'
        value={value.scale}
        min={0.1}
        max={5}
        step={0.05}
        disabled={disabled}
        onChange={(scale) => update({ scale })}
      />
      <ColorField
        label='角色颜色'
        value={value.color}
        disabled={disabled}
        onChange={(color) => update({ color })}
      />
      <section className={styles.inspectorSection}>
        <h3>姿势预设</h3>
        <div className={styles.posePresetGrid} role='listbox' aria-label='姿势预设'>
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
  const update = (patch: Partial<DirectorObjectInspectorValue>) =>
    onInspectorChange({ ...value, ...patch });

  return (
    <InspectorFrame title='物体' kind='object'>
      <label className={styles.inspectorField}>
        <span className={styles.inspectorFieldLabel}>模型名称</span>
        <Input value={value.name} disabled={disabled} onChange={(name) => update({ name })} />
      </label>
      {value.modelLabel ? (
        <label className={styles.inspectorField}>
          <span className={styles.inspectorFieldLabel}>模型来源</span>
          <Input value={value.modelLabel} readOnly />
        </label>
      ) : null}
      {value.localAssetMissing ? (
        <section className={styles.missingModel} role='alert'>
          <p>模型文件只保存在原导入设备，重新导入后会继承当前模型数据。</p>
          <Button
            icon={<Upload />}
            disabled={disabled || !onReimportObjectModel}
            onClick={onReimportObjectModel}
          >
            重新导入模型
          </Button>
        </section>
      ) : null}
      <VectorEditor
        label='模型位置'
        value={value.position}
        disabled={disabled}
        onChange={(position) => update({ position })}
      />
      <VectorEditor
        label='模型旋转'
        value={value.rotation}
        disabled={disabled}
        onChange={(rotation) => update({ rotation })}
      />
      <RangeField
        label='统一缩放'
        value={value.scale}
        min={0.1}
        max={5}
        step={0.05}
        disabled={disabled}
        onChange={(scale) => update({ scale })}
      />
      <ColorField
        label='模型颜色'
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
