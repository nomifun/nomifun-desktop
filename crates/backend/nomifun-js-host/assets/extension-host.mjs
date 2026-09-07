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

function requireMount(target) {
  const mount = mounts.get(target.mount_id);
  if (!mount) throw new Error(`Mount ${target.mount_id} is not loaded`);
  if (mount.targetKey !== targetKey(target)) {
    throw new Error("contribution target does not match the loaded Mount");
  }
  return mount;
}

function hostCall(mount, request) {
  const requestId = `js-${generation}-${nextHostRequest++}`;
  const envelope = {
    protocol_version: protocolVersion,
    host_kind: hostKind,
    host_generation: generation,
    request_id: requestId,
    direction: "java_script_to_host",
    request,
  };
  return new Promise((resolve, reject) => {
    hostRequests.set(requestId, { resolve, reject });
    writeFrame(envelope).catch((error) => {
      hostRequests.delete(requestId);
      reject(error);
    });
  });
}

function sdkFor(context) {
  const mount = {
    mountHandleId: context.mount_handle_id,
    target: structuredClone(context.target),
  };
  return Object.freeze({
    credential: Object.freeze({
      resolve: async (slotKey) => {
        const value = await hostCall(mount, {
          method: "credential_resolve",
          params: {
            mount_handle_id: mount.mountHandleId,
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
          params: { mount_handle_id: mount.mountHandleId, request },
        }),
      set: (request) =>
        hostCall(mount, {
          method: "state_set",
          params: { mount_handle_id: mount.mountHandleId, request },
        }),
      delete: (request) =>
        hostCall(mount, {
          method: "state_delete",
          params: { mount_handle_id: mount.mountHandleId, request },
        }),
      compareAndSwap: (request) =>
        hostCall(mount, {
          method: "state_compare_and_swap",
          params: { mount_handle_id: mount.mountHandleId, request },
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
      const moduleUrl = pathToFileURL(frame.module_path);
      moduleUrl.searchParams.set(
        "artifact",
        context.target.artifact_digest,
      );
      const imported = await import(moduleUrl.href);
      if (typeof imported.activate !== "function") {
        throw new Error("main.mjs must export activate(context)");
      }
      const extension = await imported.activate(
        Object.freeze({
          mount: structuredClone(context),
          sdk: sdkFor(context),
        }),
      );
      if (!extension || typeof extension !== "object") {
        throw new Error("activate(context) must return an extension object");
      }
      mounts.set(context.target.mount_id, {
        context,
        extension,
        targetKey: targetKey(context.target),
      });
      return { kind: "ack" };
    }
    case "mount_unload": {
      const mount = requireMount(params.target);
      if (typeof mount.extension.deactivate === "function") {
        await mount.extension.deactivate();
      }
      mounts.delete(params.target.mount_id);
      for (const [handle, owner] of resourceHandles) {
        if (owner.mountId === params.target.mount_id) {
          resourceHandles.delete(handle);
        }
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
      const controller = new AbortController();
      activeRequests.set(requestId, controller);
      try {
        const value = await capability.invoke(
          Object.freeze({
            actionId: params.action_id,
            input: structuredClone(params.input),
            contribution: structuredClone(params.contribution),
            signal: controller.signal,
          }),
        );
        return { kind: "value", payload: value ?? null };
      } finally {
        activeRequests.delete(requestId);
      }
    }
    case "context_contribute": {
      const mount = requireMount(params.contribution.target);
      const capability =
        mount.extension.capabilities?.[params.contribution.contribution_id];
      if (!capability || typeof capability.contributeContext !== "function") {
        throw new Error("context contribution is not implemented");
      }
      const value = await capability.contributeContext({
        schemaRef: params.schema_ref,
        contribution: structuredClone(params.contribution),
      });
      return { kind: "value", payload: value ?? null };
    }
    case "resource_acquire": {
      const mount = requireMount(params.contribution.target);
      const capability =
        mount.extension.capabilities?.[params.contribution.contribution_id];
      if (!capability || typeof capability.acquireResource !== "function") {
        throw new Error("resource contribution is not implemented");
      }
      const acquired = await capability.acquireResource({
        bindingId: params.binding_id,
        resourceKind: params.resource_kind,
        parameters: structuredClone(params.parameters),
      });
      if (
        !acquired ||
        typeof acquired.handleId !== "string" ||
        acquired.handleId.length === 0
      ) {
        throw new Error("resource acquisition must return handleId");
      }
      if (resourceHandles.has(acquired.handleId)) {
        throw new Error("resource handle ID is already active");
      }
      resourceHandles.set(acquired.handleId, {
        mountId: params.contribution.target.mount_id,
        release: acquired.release,
      });
      return {
        kind: "resource_acquired",
        payload: { handle_id: acquired.handleId },
      };
    }
    case "resource_release": {
      const resource = resourceHandles.get(params.handle_id);
      if (!resource) throw new Error("resource handle is not active");
      if (typeof resource.release === "function") await resource.release();
      resourceHandles.delete(params.handle_id);
      return { kind: "ack" };
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
          error?.name === "AbortError"
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
