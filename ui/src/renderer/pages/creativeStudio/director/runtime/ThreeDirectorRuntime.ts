/**
 * @license
 * Copyright 2025-2026 NomiFun (nomifun.com)
 * SPDX-License-Identifier: Apache-2.0
 */

import {
  AmbientLight,
  AxesHelper,
  BackSide,
  Color,
  DirectionalLight,
  Group,
  GridHelper,
  MathUtils,
  Mesh,
  MeshBasicMaterial,
  OrthographicCamera,
  PCFSoftShadowMap,
  PerspectiveCamera,
  PlaneGeometry,
  PointLight,
  Scene,
  ShadowMaterial,
  SphereGeometry,
  SpotLight,
  SRGBColorSpace,
  Texture,
  Vector2,
  Vector3,
  WebGLRenderer,
  type Camera,
  type Light,
  type Object3D,
} from 'three';
import { OrbitControls } from 'three/examples/jsm/controls/OrbitControls.js';
import { GLTFLoader } from 'three/examples/jsm/loaders/GLTFLoader.js';

import {
  DIRECTOR_LIMITS,
  clampDirectorTime,
  type DirectorCamera,
  type DirectorEvaluatedFrame,
  type DirectorLight,
  type DirectorState,
} from '../domain';
import {
  applyDirectorTransform,
  createDirectorRuntimeFramePlan,
  directorVerticalFovDegrees,
} from './scenePlan';
import {
  directorAssetResourcePath,
  disposeDirectorObject3D,
  resolveTrustedDirectorAssetUrl,
} from './resources';
import type {
  DirectorAssetUrlResolver,
  DirectorImageCaptureRequest,
  DirectorImageCaptureResult,
  DirectorRuntimeError,
  DirectorRuntimeHandle,
  DirectorRuntimeOptions,
} from './types';

interface DirectorModelBinding {
  key: string;
  group: Group;
  assetId: string | null;
  loadedRoot: Object3D | null;
  abortController: AbortController | null;
  revision: number;
}

interface DirectorCameraBinding {
  source: DirectorCamera;
  camera: PerspectiveCamera | OrthographicCamera;
}

interface DirectorLightBinding {
  source: DirectorLight;
  light: Light;
  target: Object3D | null;
}

interface LoadedPanorama {
  assetId: string;
  texture: Texture;
  closeImage: (() => void) | null;
}

class DirectorAssetLoadFailure extends Error {
  readonly code: 'asset-url' | 'asset-fetch' | 'asset-decode';
  readonly cause: unknown;

  constructor(
    code: DirectorAssetLoadFailure['code'],
    message: string,
    cause?: unknown
  ) {
    super(message);
    this.name = 'DirectorAssetLoadFailure';
    this.code = code;
    this.cause = cause;
  }
}

const DIRECTOR_CAMERA_START = new Vector3(8, 6, 10);
const DIRECTOR_CAMERA_TARGET = new Vector3(0, 1, 0);
const EMPTY_SIZE = new Vector2();

function createDomainCamera(camera: DirectorCamera): PerspectiveCamera | OrthographicCamera {
  if (camera.projection === 'orthographic') {
    const instance = new OrthographicCamera();
    instance.name = camera.name;
    return instance;
  }
  const instance = new PerspectiveCamera();
  instance.name = camera.name;
  return instance;
}

function createDomainLight(light: DirectorLight): DirectorLightBinding {
  switch (light.lightType) {
    case 'ambient':
      return { source: light, light: new AmbientLight(), target: null };
    case 'point':
      return { source: light, light: new PointLight(), target: null };
    case 'spot': {
      const target = new Group();
      const instance = new SpotLight();
      instance.target = target;
      return { source: light, light: instance, target };
    }
    case 'directional': {
      const target = new Group();
      const instance = new DirectionalLight();
      instance.target = target;
      return { source: light, light: instance, target };
    }
  }
}

function isAbortError(error: unknown): boolean {
  return (
    (typeof DOMException !== 'undefined' &&
      error instanceof DOMException &&
      error.name === 'AbortError') ||
    (error instanceof Error && error.name === 'AbortError')
  );
}

function abortError(): Error {
  return typeof DOMException === 'undefined'
    ? Object.assign(new Error('Aborted'), { name: 'AbortError' })
    : new DOMException('Aborted', 'AbortError');
}

function captureMime(format: 'png' | 'jpeg'): string {
  return format === 'png' ? 'image/png' : 'image/jpeg';
}

function assertCaptureDimension(value: number, name: string): void {
  if (!Number.isSafeInteger(value) || value <= 0 || value > DIRECTOR_LIMITS.maxCaptureDimension) {
    throw new RangeError(`${name} must be an integer between 1 and ${DIRECTOR_LIMITS.maxCaptureDimension}`);
  }
}

function canvasToBlob(
  canvas: HTMLCanvasElement,
  format: 'png' | 'jpeg'
): Promise<Blob> {
  return new Promise((resolve, reject) => {
    canvas.toBlob(
      (blob) => {
        if (blob) resolve(blob);
        else reject(new Error('The WebGL canvas could not encode the capture'));
      },
      captureMime(format),
      format === 'jpeg' ? 0.92 : undefined
    );
  });
}

function configureDomainCamera(
  binding: DirectorCameraBinding,
  viewportAspect?: number
): void {
  const { source, camera } = binding;
  const aspect = viewportAspect ?? source.aspectRatio.width / source.aspectRatio.height;
  camera.name = source.name;
  camera.near = source.nearClip;
  camera.far = source.farClip;
  camera.visible = source.visible;
  applyDirectorTransform(camera, source.transform);

  if (camera instanceof PerspectiveCamera) {
    camera.aspect = aspect;
    camera.fov = directorVerticalFovDegrees(source);
  } else {
    const halfHeight = source.orthographicSize / 2;
    camera.left = -halfHeight * aspect;
    camera.right = halfHeight * aspect;
    camera.top = halfHeight;
    camera.bottom = -halfHeight;
  }
  camera.updateProjectionMatrix();
  camera.updateMatrixWorld(true);
}

function updateDomainLight(binding: DirectorLightBinding, source: DirectorLight): void {
  const { light, target } = binding;
  binding.source = source;
  light.name = source.name;
  light.color.set(source.color);
  light.intensity = source.intensity;
  light.visible = source.visible;
  applyDirectorTransform(light, source.transform);

  if (light instanceof PointLight || light instanceof SpotLight) light.distance = source.range;
  if (light instanceof SpotLight) light.angle = MathUtils.degToRad(source.coneAngleDegrees);
  if (light instanceof PointLight || light instanceof SpotLight || light instanceof DirectionalLight) {
    light.castShadow = true;
  }
  if (target) {
    const direction = new Vector3(0, 0, -1).applyEuler(light.rotation);
    target.position.copy(light.position).add(direction);
    target.updateMatrixWorld(true);
  }
}

function disposeDomainLight(binding: DirectorLightBinding): void {
  const shadowLight = binding.light as Light & {
    shadow?: {
      map?: { dispose(): void } | null;
      mapPass?: { dispose(): void } | null;
    };
  };
  shadowLight.shadow?.map?.dispose();
  shadowLight.shadow?.mapPass?.dispose();
  binding.light.removeFromParent();
  binding.target?.removeFromParent();
}

export class ThreeDirectorRuntime implements DirectorRuntimeHandle {
  readonly canvas: HTMLCanvasElement;

  private readonly container: HTMLElement;
  private readonly scene = new Scene();
  private readonly worldRoot = new Group();
  private readonly renderer: WebGLRenderer;
  private readonly directorCamera = new PerspectiveCamera(45, 1, 0.01, 100_000);
  private readonly controls: OrbitControls;
  private readonly gltfLoader = new GLTFLoader();
  private readonly grid = new GridHelper(100, 100, 0x595c66, 0x343741);
  private readonly axes = new AxesHelper(1.5);
  private readonly ground = new Mesh(
    new PlaneGeometry(100, 100),
    new ShadowMaterial({ color: 0x000000, opacity: 0.2 })
  );
  private readonly panoramaMaterial = new MeshBasicMaterial({
    side: BackSide,
    depthWrite: false,
    toneMapped: false,
  });
  private readonly panoramaSphere = new Mesh(
    new SphereGeometry(1, 64, 32),
    this.panoramaMaterial
  );
  private readonly modelBindings = new Map<string, DirectorModelBinding>();
  private readonly cameraBindings = new Map<string, DirectorCameraBinding>();
  private readonly lightBindings = new Map<string, DirectorLightBinding>();
  private readonly maxPixelRatio: number;
  private readonly showAxes: boolean;
  private resizeObserver: ResizeObserver | null = null;
  private resolveAssetUrl: DirectorAssetUrlResolver;
  private onError: ((error: DirectorRuntimeError) => void) | undefined;
  private currentState: DirectorState | null = null;
  private currentFrame: DirectorEvaluatedFrame | null = null;
  private renderCamera: Camera;
  private playbackTime = 0;
  private previousAnimationTime: number | null = null;
  private animationFrame: number | null = null;
  private running = false;
  private disposed = false;
  private captureInProgress = false;
  private captureQueue: Promise<void> = Promise.resolve();
  private panorama: LoadedPanorama | null = null;
  private panoramaAssetId: string | null = null;
  private panoramaAbortController: AbortController | null = null;
  private panoramaRevision = 0;

  constructor(options: DirectorRuntimeOptions) {
    this.container = options.container;
    this.resolveAssetUrl = options.resolveAssetUrl;
    this.onError = options.onError;
    this.maxPixelRatio = Math.max(1, options.maxPixelRatio ?? 2);
    this.showAxes = options.showAxes ?? true;

    this.renderer = new WebGLRenderer({
      antialias: true,
      alpha: false,
      preserveDrawingBuffer: true,
      powerPreference: 'high-performance',
    });
    this.renderer.outputColorSpace = SRGBColorSpace;
    this.renderer.shadowMap.enabled = true;
    this.renderer.shadowMap.type = PCFSoftShadowMap;
    this.canvas = this.renderer.domElement;
    this.canvas.dataset.directorRuntimeCanvas = '';
    this.canvas.style.display = 'block';
    this.canvas.style.width = '100%';
    this.canvas.style.height = '100%';
    this.canvas.style.touchAction = 'none';
    this.canvas.addEventListener('webglcontextlost', this.handleContextLost);
    this.canvas.addEventListener('webglcontextrestored', this.handleContextRestored);

    this.scene.name = 'NomiFun Director';
    this.scene.add(this.worldRoot);
    this.worldRoot.name = 'Director Scene';
    this.grid.name = 'Director Grid';
    this.axes.name = 'Director Axes';
    this.ground.name = 'Director Ground Shadow';
    this.ground.rotation.x = -Math.PI / 2;
    this.ground.receiveShadow = true;
    this.panoramaSphere.name = 'Director Panorama';
    this.panoramaSphere.renderOrder = -1_000;
    this.panoramaSphere.frustumCulled = false;
    this.panoramaSphere.visible = false;
    this.worldRoot.add(this.ground, this.grid, this.axes, this.panoramaSphere);

    this.directorCamera.name = 'Director View Camera';
    this.directorCamera.position.copy(DIRECTOR_CAMERA_START);
    this.directorCamera.lookAt(DIRECTOR_CAMERA_TARGET);
    this.scene.add(this.directorCamera);
    this.renderCamera = this.directorCamera;

    this.container.appendChild(this.canvas);
    this.controls = new OrbitControls(this.directorCamera, this.canvas);
    this.controls.target.copy(DIRECTOR_CAMERA_TARGET);
    this.controls.enableDamping = true;
    this.controls.dampingFactor = 0.08;
    this.controls.update();

    this.resize();
    if (typeof ResizeObserver !== 'undefined') {
      this.resizeObserver = new ResizeObserver(() => this.resize());
      this.resizeObserver.observe(this.container);
    }
    this.start();
  }

  update(state: DirectorState, timeSeconds = state.timeline.currentTimeSeconds): void {
    this.assertAlive();
    this.currentState = state;
    this.playbackTime = clampDirectorTime(timeSeconds, state.timeline.durationSeconds);
    this.applyFrame();
  }

  setAssetUrlResolver(resolveAssetUrl: DirectorAssetUrlResolver): void {
    this.assertAlive();
    if (this.resolveAssetUrl === resolveAssetUrl) return;
    this.resolveAssetUrl = resolveAssetUrl;
    for (const binding of this.modelBindings.values()) {
      if (binding.assetId) this.beginModelLoad(binding, binding.assetId, true);
    }
    if (this.panoramaAssetId) this.beginPanoramaLoad(this.panoramaAssetId, true);
  }

  setErrorHandler(onError: DirectorRuntimeOptions['onError']): void {
    this.onError = onError;
  }

  resize(): void {
    if (this.disposed) return;
    const width = Math.max(1, Math.floor(this.container.clientWidth || 1));
    const height = Math.max(1, Math.floor(this.container.clientHeight || 1));
    const devicePixelRatio = globalThis.devicePixelRatio || 1;
    this.renderer.setPixelRatio(Math.min(this.maxPixelRatio, devicePixelRatio));
    this.renderer.setSize(width, height, false);
    this.directorCamera.aspect = width / height;
    this.directorCamera.updateProjectionMatrix();
  }

  start(): void {
    this.assertAlive();
    if (this.running) return;
    this.running = true;
    this.previousAnimationTime = null;
    this.animationFrame = requestAnimationFrame(this.animate);
  }

  stop(): void {
    this.running = false;
    this.previousAnimationTime = null;
    if (this.animationFrame !== null) {
      cancelAnimationFrame(this.animationFrame);
      this.animationFrame = null;
    }
  }

  captureImage(request: DirectorImageCaptureRequest): Promise<DirectorImageCaptureResult> {
    const capture = this.captureQueue.then(() => this.captureImageNow(request));
    this.captureQueue = capture.then(
      () => undefined,
      () => undefined
    );
    return capture;
  }

  dispose(): void {
    if (this.disposed) return;
    this.disposed = true;
    this.stop();
    this.resizeObserver?.disconnect();
    this.resizeObserver = null;
    this.controls.dispose();
    this.panoramaAbortController?.abort();
    this.panoramaAbortController = null;
    this.clearPanorama();

    for (const binding of this.modelBindings.values()) this.disposeModelBinding(binding);
    this.modelBindings.clear();
    for (const binding of this.lightBindings.values()) disposeDomainLight(binding);
    this.lightBindings.clear();
    for (const binding of this.cameraBindings.values()) binding.camera.removeFromParent();
    this.cameraBindings.clear();

    disposeDirectorObject3D(this.ground);
    disposeDirectorObject3D(this.grid);
    disposeDirectorObject3D(this.axes);
    disposeDirectorObject3D(this.panoramaSphere);
    this.scene.clear();
    this.renderer.renderLists.dispose();
    this.renderer.dispose();
    this.canvas.removeEventListener('webglcontextlost', this.handleContextLost);
    this.canvas.removeEventListener('webglcontextrestored', this.handleContextRestored);
    this.renderer.forceContextLoss();
    if (this.canvas.parentElement === this.container) this.container.removeChild(this.canvas);
  }

  private readonly animate = (time: number): void => {
    if (!this.running || this.disposed) return;
    const deltaSeconds =
      this.previousAnimationTime === null
        ? 0
        : Math.min(0.1, Math.max(0, (time - this.previousAnimationTime) / 1_000));
    this.previousAnimationTime = time;

    const timeline = this.currentState?.timeline;
    if (this.currentState && timeline?.playing && timeline.durationSeconds > 0) {
      const next = this.playbackTime + deltaSeconds;
      this.playbackTime = timeline.loop
        ? next % timeline.durationSeconds
        : Math.min(timeline.durationSeconds, next);
      this.applyFrame();
    }

    this.controls.update();
    if (!this.captureInProgress) this.renderer.render(this.scene, this.renderCamera);
    this.animationFrame = requestAnimationFrame(this.animate);
  };

  private readonly handleContextLost = (event: Event): void => {
    event.preventDefault();
    if (this.disposed) return;
    this.report({
      code: 'renderer',
      message: 'The Director WebGL context was lost',
    });
  };

  private readonly handleContextRestored = (): void => {
    if (this.disposed) return;
    this.resize();
    this.applyFrame();
  };

  private applyFrame(): void {
    if (!this.currentState) return;
    const plan = createDirectorRuntimeFramePlan(this.currentState, this.playbackTime);
    const { frame } = plan;
    this.currentFrame = frame;
    applyDirectorTransform(this.worldRoot, frame.scene.transform);
    this.scene.background = new Color(frame.scene.environment.skyColor);
    this.grid.visible = frame.scene.environment.gridVisible;
    this.ground.visible = frame.scene.environment.groundVisible;
    this.axes.visible = this.showAxes && !plan.useActiveCamera;
    this.syncPanorama(frame);
    this.syncCameras(frame);
    this.syncModels(frame);
    this.syncLights(frame);

    const activeCamera = plan.activeCameraId
      ? this.cameraBindings.get(plan.activeCameraId)?.camera
      : undefined;
    this.renderCamera = plan.useActiveCamera && activeCamera ? activeCamera : this.directorCamera;
    this.controls.enabled = this.renderCamera === this.directorCamera;
  }

  private syncCameras(frame: DirectorEvaluatedFrame): void {
    const retained = new Set<string>();
    for (const source of frame.cameras) {
      retained.add(source.id);
      let binding = this.cameraBindings.get(source.id);
      if (!binding || binding.source.projection !== source.projection) {
        binding?.camera.removeFromParent();
        binding = { source, camera: createDomainCamera(source) };
        this.cameraBindings.set(source.id, binding);
        this.worldRoot.add(binding.camera);
      }
      binding.source = source;
      configureDomainCamera(binding);
    }
    for (const [id, binding] of this.cameraBindings) {
      if (retained.has(id)) continue;
      binding.camera.removeFromParent();
      this.cameraBindings.delete(id);
    }
  }

  private syncModels(frame: DirectorEvaluatedFrame): void {
    const sources = [
      ...frame.characters.map((entity) => ({ entity, key: `character:${entity.id}` })),
      ...frame.objects.map((entity) => ({ entity, key: `object:${entity.id}` })),
    ];
    const retained = new Set<string>();

    for (const { entity, key } of sources) {
      retained.add(key);
      let binding = this.modelBindings.get(key);
      if (!binding) {
        const group = new Group();
        group.name = entity.name;
        this.worldRoot.add(group);
        binding = {
          key,
          group,
          assetId: null,
          loadedRoot: null,
          abortController: null,
          revision: 0,
        };
        this.modelBindings.set(key, binding);
      }
      binding.group.name = entity.name;
      binding.group.visible = entity.visible;
      applyDirectorTransform(binding.group, entity.transform);
      const assetId = entity.asset?.assetId ?? null;
      if (assetId !== binding.assetId) {
        if (assetId) this.beginModelLoad(binding, assetId, false);
        else this.clearModelAsset(binding);
      }
    }

    for (const [key, binding] of this.modelBindings) {
      if (retained.has(key)) continue;
      this.disposeModelBinding(binding);
      this.modelBindings.delete(key);
    }
  }

  private syncLights(frame: DirectorEvaluatedFrame): void {
    const retained = new Set<string>();
    for (const source of frame.lights) {
      retained.add(source.id);
      let binding = this.lightBindings.get(source.id);
      if (!binding || binding.source.lightType !== source.lightType) {
        if (binding) disposeDomainLight(binding);
        binding = createDomainLight(source);
        this.lightBindings.set(source.id, binding);
        this.worldRoot.add(binding.light);
        if (binding.target) this.worldRoot.add(binding.target);
      }
      updateDomainLight(binding, source);
    }
    for (const [id, binding] of this.lightBindings) {
      if (retained.has(id)) continue;
      disposeDomainLight(binding);
      this.lightBindings.delete(id);
    }
  }

  private syncPanorama(frame: DirectorEvaluatedFrame): void {
    const environment = frame.scene.environment;
    this.panoramaSphere.rotation.y = MathUtils.degToRad(environment.panoramaYawDegrees);
    this.panoramaSphere.scale.setScalar(environment.panoramaRadius);
    const assetId = environment.panorama?.assetId ?? null;
    if (assetId === this.panoramaAssetId) return;
    if (assetId) this.beginPanoramaLoad(assetId, false);
    else {
      this.panoramaAssetId = null;
      this.panoramaAbortController?.abort();
      this.panoramaAbortController = null;
      this.clearPanorama();
    }
  }

  private beginModelLoad(
    binding: DirectorModelBinding,
    assetId: string,
    force: boolean
  ): void {
    if (!force && binding.assetId === assetId) return;
    this.clearModelAsset(binding);
    binding.assetId = assetId;
    binding.revision += 1;
    const revision = binding.revision;
    const abortController = new AbortController();
    binding.abortController = abortController;

    void this.loadModel(assetId, abortController.signal)
      .then((root) => {
        if (
          this.disposed ||
          abortController.signal.aborted ||
          binding.revision !== revision ||
          binding.assetId !== assetId
        ) {
          disposeDirectorObject3D(root);
          return;
        }
        binding.abortController = null;
        binding.loadedRoot = root;
        binding.group.add(root);
      })
      .catch((error) => {
        if (isAbortError(error)) return;
        binding.abortController = null;
        const failure =
          error instanceof DirectorAssetLoadFailure
            ? error
            : new DirectorAssetLoadFailure(
                'asset-decode',
                `Unable to decode Director model asset ${assetId}`,
                error
              );
        this.report({
          code: failure.code,
          message: failure.message,
          assetId,
          cause: failure.cause,
        });
      });
  }

  private async loadModel(assetId: string, signal: AbortSignal): Promise<Object3D> {
    const url = await this.resolveUrl(assetId, signal);
    let response: Response;
    try {
      response = await fetch(url, { credentials: 'include', signal });
    } catch (error) {
      if (isAbortError(error)) throw error;
      throw new DirectorAssetLoadFailure(
        'asset-fetch',
        `Unable to fetch Director model asset ${assetId}`,
        error
      );
    }
    if (!response.ok) {
      throw new DirectorAssetLoadFailure(
        'asset-fetch',
        `Model asset request failed with HTTP ${response.status}`
      );
    }
    let data: ArrayBuffer;
    try {
      data = await response.arrayBuffer();
    } catch (error) {
      if (isAbortError(error)) throw error;
      throw new DirectorAssetLoadFailure(
        'asset-fetch',
        `Unable to read Director model asset ${assetId}`,
        error
      );
    }
    if (signal.aborted) throw abortError();
    let gltf: Awaited<ReturnType<GLTFLoader['parseAsync']>>;
    try {
      gltf = await this.gltfLoader.parseAsync(data, directorAssetResourcePath(url));
    } catch (error) {
      throw new DirectorAssetLoadFailure(
        'asset-decode',
        `Unable to decode Director model asset ${assetId}`,
        error
      );
    }
    gltf.scene.traverse((object) => {
      const mesh = object as Object3D & { isMesh?: boolean; castShadow?: boolean; receiveShadow?: boolean };
      if (!mesh.isMesh) return;
      mesh.castShadow = true;
      mesh.receiveShadow = true;
    });
    return gltf.scene;
  }

  private clearModelAsset(binding: DirectorModelBinding): void {
    binding.abortController?.abort();
    binding.abortController = null;
    binding.revision += 1;
    if (binding.loadedRoot) disposeDirectorObject3D(binding.loadedRoot);
    binding.loadedRoot = null;
    binding.assetId = null;
  }

  private disposeModelBinding(binding: DirectorModelBinding): void {
    this.clearModelAsset(binding);
    binding.group.removeFromParent();
  }

  private beginPanoramaLoad(assetId: string, force: boolean): void {
    if (!force && assetId === this.panoramaAssetId) return;
    this.panoramaAssetId = assetId;
    this.panoramaAbortController?.abort();
    this.clearPanorama();
    this.panoramaRevision += 1;
    const revision = this.panoramaRevision;
    const abortController = new AbortController();
    this.panoramaAbortController = abortController;

    void this.loadPanorama(assetId, abortController.signal)
      .then((panorama) => {
        if (
          this.disposed ||
          abortController.signal.aborted ||
          revision !== this.panoramaRevision ||
          this.panoramaAssetId !== assetId
        ) {
          panorama.texture.dispose();
          panorama.closeImage?.();
          return;
        }
        this.panoramaAbortController = null;
        this.panorama = panorama;
        this.panoramaMaterial.map = panorama.texture;
        this.panoramaMaterial.needsUpdate = true;
        this.panoramaSphere.visible = true;
      })
      .catch((error) => {
        if (isAbortError(error)) return;
        this.panoramaAbortController = null;
        const failure =
          error instanceof DirectorAssetLoadFailure
            ? error
            : new DirectorAssetLoadFailure(
                'asset-decode',
                `Unable to decode Director panorama asset ${assetId}`,
                error
              );
        this.report({
          code: failure.code,
          message: failure.message,
          assetId,
          cause: failure.cause,
        });
      });
  }

  private async loadPanorama(assetId: string, signal: AbortSignal): Promise<LoadedPanorama> {
    const url = await this.resolveUrl(assetId, signal);
    let response: Response;
    try {
      response = await fetch(url, { credentials: 'include', signal });
    } catch (error) {
      if (isAbortError(error)) throw error;
      throw new DirectorAssetLoadFailure(
        'asset-fetch',
        `Unable to fetch Director panorama asset ${assetId}`,
        error
      );
    }
    if (!response.ok) {
      throw new DirectorAssetLoadFailure(
        'asset-fetch',
        `Panorama asset request failed with HTTP ${response.status}`
      );
    }
    let blob: Blob;
    try {
      blob = await response.blob();
    } catch (error) {
      if (isAbortError(error)) throw error;
      throw new DirectorAssetLoadFailure(
        'asset-fetch',
        `Unable to read Director panorama asset ${assetId}`,
        error
      );
    }
    if (signal.aborted) throw abortError();

    if (typeof createImageBitmap === 'function') {
      const bitmap = await createImageBitmap(blob);
      if (signal.aborted) {
        bitmap.close();
        throw abortError();
      }
      const texture = new Texture(bitmap);
      texture.colorSpace = SRGBColorSpace;
      texture.needsUpdate = true;
      return { assetId, texture, closeImage: () => bitmap.close() };
    }

    const objectUrl = URL.createObjectURL(blob);
    try {
      const image = await new Promise<HTMLImageElement>((resolve, reject) => {
        const element = new Image();
        element.onload = () => resolve(element);
        element.onerror = () => reject(new Error('Panorama image decoding failed'));
        element.src = objectUrl;
      });
      if (signal.aborted) throw abortError();
      const texture = new Texture(image);
      texture.colorSpace = SRGBColorSpace;
      texture.needsUpdate = true;
      return { assetId, texture, closeImage: null };
    } finally {
      URL.revokeObjectURL(objectUrl);
    }
  }

  private clearPanorama(): void {
    if (this.panorama) {
      this.panorama.texture.dispose();
      this.panorama.closeImage?.();
      this.panorama = null;
    }
    this.panoramaMaterial.map = null;
    this.panoramaMaterial.needsUpdate = true;
    this.panoramaSphere.visible = false;
  }

  private async resolveUrl(assetId: string, signal: AbortSignal): Promise<string> {
    try {
      const value = await this.resolveAssetUrl(assetId, signal);
      if (signal.aborted) throw abortError();
      if (value === null) {
        throw new TypeError(`No trusted URL is available for asset ${assetId}`);
      }
      return resolveTrustedDirectorAssetUrl(value);
    } catch (error) {
      if (isAbortError(error)) throw error;
      throw new DirectorAssetLoadFailure(
        'asset-url',
        `Unable to resolve a trusted URL for Director asset ${assetId}`,
        error
      );
    }
  }

  private captureImageNow = async (
    request: DirectorImageCaptureRequest
  ): Promise<DirectorImageCaptureResult> => {
    this.assertAlive();
    if (request.kind !== 'image') throw new TypeError('Only image capture is supported by this API');
    assertCaptureDimension(request.width, 'capture width');
    assertCaptureDimension(request.height, 'capture height');
    const binding = this.cameraBindings.get(request.cameraId);
    if (!binding) throw new Error(`Director camera ${request.cameraId} does not exist`);

    const previousSize = this.renderer.getSize(EMPTY_SIZE.clone());
    const previousPixelRatio = this.renderer.getPixelRatio();
    const previousGridVisibility = this.grid.visible;
    const previousAxesVisibility = this.axes.visible;
    this.captureInProgress = true;
    try {
      this.grid.visible = false;
      this.axes.visible = false;
      this.renderer.setPixelRatio(1);
      this.renderer.setSize(request.width, request.height, false);
      configureDomainCamera(binding, request.width / request.height);
      this.renderer.render(this.scene, binding.camera);
      const blob = await canvasToBlob(this.canvas, request.format);
      return {
        requestId: request.requestId,
        cameraId: request.cameraId,
        width: request.width,
        height: request.height,
        format: request.format,
        blob,
      };
    } catch (error) {
      this.report({
        code: 'capture',
        message: `Unable to capture Director camera ${request.cameraId}`,
        cause: error,
      });
      throw error;
    } finally {
      if (!this.disposed) {
        this.renderer.setPixelRatio(previousPixelRatio);
        this.renderer.setSize(previousSize.x, previousSize.y, false);
        configureDomainCamera(binding);
      }
      this.grid.visible = previousGridVisibility;
      this.axes.visible = previousAxesVisibility;
      this.captureInProgress = false;
    }
  };

  private report(error: DirectorRuntimeError): void {
    try {
      this.onError?.(error);
    } catch (handlerError) {
      console.error('[ThreeDirectorRuntime] error handler failed', handlerError);
    }
  }

  private assertAlive(): void {
    if (this.disposed) throw new Error('Director runtime is disposed');
  }
}
