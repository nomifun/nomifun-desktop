import readline from "node:readline";
import { pathToFileURL } from "node:url";

const bootstrap = JSON.parse(
  Buffer.from(process.argv[2] ?? "", "hex").toString("utf8"),
);
if (
  !bootstrap ||
  !["shared_extension", "candidate_test"].includes(bootstrap.host_kind) ||
  !Number.isSafeInteger(bootstrap.host_generation) ||
  bootstrap.host_generation <= 0 ||
  !bootstrap.runtime ||
  !Array.isArray(bootstrap.supported_methods)
) {
  throw new Error("invalid JavaScript Extension Host bootstrap");
}

const generation = bootstrap.host_generation;
const hostKind = bootstrap.host_kind;
const protocolVersion = bootstrap.runtime.javascript_host_protocol_version;
const mounts = new Map();
const resourceHandles = new Map();
const activeRequests = new Map();
const hostRequests = new Map();
let nextHostRequest = 1;
let nextResourceHandle = 1n;
let shuttingDown = false;
let writeChain = Promise.resolve();

function writeFrame(frame) {
  const line = `${JSON.stringify(frame)}\n`;
  writeChain = writeChain.then(
    () =>
      new Promise((resolve, reject) => {
        process.stdout.write(line, (error) => {
          if (error) reject(error);
          else resolve();
        });
      }),
  );
  return writeChain;
}

await writeFrame({
  schema_version: "1.0.0",
  host_kind: hostKind,
  host_generation: generation,
  process_id: process.pid,
  runtime: bootstrap.runtime,
  supported_methods: bootstrap.supported_methods,
});

process.on("uncaughtException", (error) => {
  process.stderr.write(`uncaught exception: ${String(error)}\n`);
  process.exit(70);
});
process.on("unhandledRejection", (error) => {
  process.stderr.write(`unhandled rejection: ${String(error)}\n`);
  process.exit(71);
});

function success(requestId, payload) {
  return {
    protocol_version: protocolVersion,
    host_kind: hostKind,
    host_generation: generation,
    request_id: requestId,
    response: { outcome: "success", value: payload },
  };
}

function failure(requestId, code, message, retryable = false) {
  return {
    protocol_version: protocolVersion,
    host_kind: hostKind,
    host_generation: generation,
    request_id: requestId,
    response: {
      outcome: "failure",
      value: { code, message: String(message), retryable },
    },
  };
}

function targetKey(target) {
  return JSON.stringify(target);
}

class CleanupError extends Error {
  constructor() { super("Plugin cleanup failed"); }
}

function requireMount(target) {
  const mount = mounts.get(target.mount_id);
  if (!mount) throw new Error(`Mount ${target.mount_id} is not loaded`);
  if (mount.targetKey !== targetKey(target)) {
    throw new Error("contribution target does not match the loaded Mount");
  }
  if (mount.phase !== "active") throw new Error("Mount is not accepting requests");
  return mount;
}

async function runMountRequest(mount, action) {
  if (mount.phase !== "active") throw new Error("Mount is not accepting requests");
  mount.activeRequests += 1;
  try {
    return await action();
  } finally {
    mount.activeRequests -= 1;
  }
}

async function closeMount(mount) {
  // Retained SDK closures must not bind to a later activation with the same
  // mount_handle_id. Drain calls already sent before publishing MountLoad or
  // MountUnload completion; Rust still owns their exact context until then.
  mount.phase = "closed";
  await Promise.allSettled([...mount.services]);
  mounts.delete(mount.context.target.mount_id);
}

async function releaseResource(resource) {
  if (!resource.releasing) {
    resource.releasing = (async () => {
      if (typeof resource.release === "function") await resource.release();
      resourceHandles.delete(resource.handleId);
      resource.mount.resources.delete(resource.localId);
    })().finally(() => { resource.releasing = null; });
  }
  await resource.releasing;
}

async function hostCall(mount, request) {
  if (mount.phase === "closed") throw new Error("Mount SDK is no longer active");
  const requestId = `js-${generation}-${nextHostRequest++}`;
  const envelope = {
    protocol_version: protocolVersion,
    host_kind: hostKind,
    host_generation: generation,
    request_id: requestId,
    direction: "java_script_to_host",
    request,
  };
  const pending = new Promise((resolve, reject) => {
    hostRequests.set(requestId, { resolve, reject });
    writeFrame(envelope).catch((error) => {
      hostRequests.delete(requestId);
      reject(error);
    });
  });
  mount.services.add(pending);
  try {
    return await pending;
  } finally {
    mount.services.delete(pending);
  }
}

function sdkFor(mount) {
  const mountHandleId = mount.context.mount_handle_id;
  return Object.freeze({
    credential: Object.freeze({
      resolve: async (slotKey) => {
        const value = await hostCall(mount, {
          method: "credential_resolve",
          params: {
            mount_handle_id: mountHandleId,
            slot_key: slotKey,
          },
        });
        return value.secret;
      },
    }),
    state: Object.freeze({
      get: (request) =>
        hostCall(mount, {
          method: "state_get",
          params: { mount_handle_id: mountHandleId, request },
        }),
      set: (request) =>
        hostCall(mount, {
          method: "state_set",
          params: { mount_handle_id: mountHandleId, request },
        }),
      delete: (request) =>
        hostCall(mount, {
          method: "state_delete",
          params: { mount_handle_id: mountHandleId, request },
        }),
      compareAndSwap: (request) =>
        hostCall(mount, {
          method: "state_compare_and_swap",
          params: { mount_handle_id: mountHandleId, request },
        }),
    }),
  });
}

async function dispatch(frame) {
  const envelope = frame.envelope;
  if (
    frame.kind !== "request" ||
    envelope.host_kind !== hostKind ||
    envelope.host_generation !== generation ||
    envelope.direction !== "host_to_java_script"
  ) {
    throw new Error("request does not bind the active Host generation");
  }

  const { request_id: requestId, request } = envelope;
  const params = request.params ?? {};
  switch (request.method) {
    case "mount_load": {
      const context = params.context;
      if (!frame.module_path || mounts.has(context.target.mount_id)) {
        throw new Error("MountLoad requires one new immutable module path");
      }
      const mount = {
        context, targetKey: targetKey(context.target), phase: "loading",
        activeRequests: 0, services: new Set(), resources: new Map(), extension: null,
      };
      mounts.set(context.target.mount_id, mount);
      try {
        const moduleUrl = pathToFileURL(frame.module_path);
        moduleUrl.searchParams.set("artifact", context.target.artifact_digest);
        const imported = await import(moduleUrl.href);
        if (typeof imported.activate !== "function") {
          throw new Error("main.mjs must export activate(context)");
        }
        const extension = await imported.activate(Object.freeze({
          mount: structuredClone(context), sdk: sdkFor(mount),
        }));
        if (!extension || typeof extension !== "object") {
          throw new Error("activate(context) must return an extension object");
        }
        mount.extension = extension;
        mount.phase = "active";
      } catch (error) {
        await closeMount(mount);
        throw error;
      }
      return { kind: "ack" };
    }
    case "mount_unload": {
      const mount = requireMount(params.target);
      if (mount.activeRequests || mount.services.size) {
        throw new Error("Mount has outstanding requests or SDK calls");
      }
      mount.phase = "unloading";
      let failures = 0;
      for (const resource of mount.resources.values()) {
        try { await releaseResource(resource); } catch { failures += 1; }
      }
      try {
        if (typeof mount.extension.deactivate === "function") await mount.extension.deactivate();
      } catch { failures += 1; }
      await closeMount(mount);
      if (failures) {
        // Disposal may have partially mutated the extension. Do not advertise
        // a usable Mount or retry callbacks against unknown state.
        throw new CleanupError();
      }
      return { kind: "ack" };
    }
    case "capability_invoke": {
      const mount = requireMount(params.contribution.target);
      const capability =
        mount.extension.capabilities?.[params.contribution.contribution_id];
      if (!capability || typeof capability.invoke !== "function") {
        throw new Error("capability contribution is not implemented");
      }
      return runMountRequest(mount, async () => {
        const controller = new AbortController();
        activeRequests.set(requestId, controller);
        try {
          const value = await capability.invoke(Object.freeze({
            actionId: params.action_id,
            input: structuredClone(params.input),
            contribution: structuredClone(params.contribution),
            signal: controller.signal,
          }));
          return { kind: "value", payload: value ?? null };
        } finally {
          activeRequests.delete(requestId);
        }
      });
    }
    case "context_contribute": {
      const mount = requireMount(params.contribution.target);
      const capability =
        mount.extension.capabilities?.[params.contribution.contribution_id];
      if (!capability || typeof capability.contributeContext !== "function") {
        throw new Error("context contribution is not implemented");
      }
      return runMountRequest(mount, async () => {
        const value = await capability.contributeContext({
          schemaRef: params.schema_ref,
          contribution: structuredClone(params.contribution),
        });
        return { kind: "value", payload: value ?? null };
      });
    }
    case "resource_acquire": {
      const mount = requireMount(params.contribution.target);
      const capability =
        mount.extension.capabilities?.[params.contribution.contribution_id];
      if (!capability || typeof capability.acquireResource !== "function") {
        throw new Error("resource contribution is not implemented");
      }
      return runMountRequest(mount, async () => {
        const acquired = await capability.acquireResource({
          bindingId: params.binding_id,
          resourceKind: params.resource_kind,
          parameters: structuredClone(params.parameters),
        });
        const release = typeof acquired?.release === "function" ? acquired.release.bind(acquired) : undefined;
        const rejection = !acquired || typeof acquired.handleId !== "string" || !acquired.handleId.length
          ? "resource acquisition must return handleId"
          : mount.resources.has(acquired.handleId) ? "resource handle ID is already active in this Mount" : null;
        if (rejection) {
          // Even a rejected acquisition transfers cleanup responsibility to us.
          // Dispose that returned value, never the already registered owner.
          try { await release?.(); } catch { throw new CleanupError(); }
          throw new Error(rejection);
        }
        // The plugin ID is local to an acquisition, not a durable release
        // capability. Wire IDs never repeat, even after release or remount.
        const handleId = "resource-" + generation + "-" + nextResourceHandle++;
        const resource = {
          mount, handleId, localId: acquired.handleId, release, releasing: null,
        };
        mount.resources.set(acquired.handleId, resource);
        resourceHandles.set(handleId, resource);
        return { kind: "resource_acquired", payload: { handle_id: handleId } };
      });
    }
    case "resource_release": {
      const resource = resourceHandles.get(params.handle_id);
      if (!resource) return { kind: "ack" };
      return runMountRequest(resource.mount, async () => {
        await releaseResource(resource);
        return { kind: "ack" };
      });
    }
    case "request_cancel": {
      activeRequests.get(params.target_request_id)?.abort();
      return { kind: "ack" };
    }
    case "host_shutdown": {
      shuttingDown = true;
      return { kind: "ack" };
    }
    default:
      throw new Error(`unsupported Extension Host method ${request.method}`);
  }
}

function acceptHostResponse(response) {
  if (
    response.host_kind !== hostKind ||
    response.host_generation !== generation
  ) {
    throw new Error("Host service response generation mismatch");
  }
  const pending = hostRequests.get(response.request_id);
  if (!pending) throw new Error("unknown Host service response");
  hostRequests.delete(response.request_id);
  if (response.response.outcome === "success") {
    pending.resolve(response.response.value.payload);
  } else {
    pending.reject(
      new Error(
        `${response.response.value.code}: ${response.response.value.message}`,
      ),
    );
  }
}

const lines = readline.createInterface({
  input: process.stdin,
  crlfDelay: Infinity,
  terminal: false,
});

lines.on("line", (line) => {
  let frame;
  try {
    frame = JSON.parse(line);
  } catch (error) {
    throw new Error(`invalid NDJSON frame: ${String(error)}`);
  }
  if (frame.response) {
    acceptHostResponse(frame);
    return;
  }
  void dispatch(frame)
    .then((payload) => writeFrame(success(frame.envelope.request_id, payload)))
    .then(() => {
      if (shuttingDown) {
        process.stdin.pause();
      }
    })
    .catch((error) =>
      writeFrame(
        failure(
          frame.envelope?.request_id ?? "invalid-request",
          error instanceof CleanupError
            ? "PLUGIN_CLEANUP_FAILED"
            : error?.name === "AbortError"
              ? "REQUEST_CANCELED"
              : "PLUGIN_INVOCATION_FAILED",
          error?.message ?? String(error),
          false,
        ),
      ),
    );
});

lines.on("close", () => {
  if (!shuttingDown) process.exitCode = 72;
});
