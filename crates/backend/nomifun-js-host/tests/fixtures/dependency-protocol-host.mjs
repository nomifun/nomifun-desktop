import readline from "node:readline";
const bootstrap = JSON.parse(Buffer.from(process.argv[2], "hex").toString("utf8"));
const write = (value) => process.stdout.write(JSON.stringify(value) + "\n");
const binding = { protocol_version: bootstrap.runtime.javascript_host_protocol_version,
  host_kind: bootstrap.host_kind, host_generation: bootstrap.host_generation };
const reply = (id, value) => write({ ...binding, request_id: id, response: { outcome: "success", value } });
const mounts = new Map();
const parents = new Map();
let previousParent;
write({ schema_version: "1.0.0", host_kind: bootstrap.host_kind, host_generation: bootstrap.host_generation,
  process_id: process.pid, runtime: bootstrap.runtime, supported_methods: bootstrap.supported_methods });
readline.createInterface({ input: process.stdin, crlfDelay: Infinity }).on("line", (line) => {
  const frame = JSON.parse(line);
  if (frame.response) {
    const parent = parents.get(frame.request_id);
    parents.delete(frame.request_id);
    reply(parent, { kind: "value", payload: frame.response });
    return;
  }
  const { request, request_id: id } = frame.envelope;
  if (request.method === "mount_load") { mounts.set(request.params.context.target.mount_id, request.params.context); reply(id, { kind: "ack" }); return; }
  if (request.method === "host_shutdown") { reply(id, { kind: "ack" }); return; }
  if (!["capability_invoke", "context_contribute"].includes(request.method)) throw new Error("unexpected fixture method");
  const mode = request.method === "context_contribute"
    ? JSON.parse(request.params.input.turn.text).input.mode : request.params.input.mode;
  const mount = mounts.get(mode === "wrong-mount" ? "mount-b" : request.params.contribution.target.mount_id);
  const childId = `child:${id}`;
  parents.set(childId, id);
  write({ ...binding, host_generation: mode === "wrong-generation" ? binding.host_generation + 1 : binding.host_generation,
    request_id: childId, direction: "java_script_to_host", request: { method: "dependency_invoke", params: {
      mount_handle_id: mount.mount_handle_id,
      parent_request_id: mode === "unknown-parent" ? "unknown" : mode === "stale-parent" ? previousParent : id,
      call: { capability_id: "fixture.child", action_id: "run", call_key: "one", input: {} },
    } } });
  previousParent = id;
});
