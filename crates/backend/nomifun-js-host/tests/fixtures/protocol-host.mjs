import readline from "node:readline";
const bootstrap = JSON.parse(Buffer.from(process.argv[2], "hex").toString("utf8"));
const write = (value) => process.stdout.write(JSON.stringify(value) + "\n");
const binding = {
  protocol_version: bootstrap.runtime.javascript_host_protocol_version,
  host_kind: bootstrap.host_kind,
  host_generation: bootstrap.host_generation,
};
const ack = (id) => ({ ...binding, request_id: id, response: { outcome: "success", value: { kind: "ack" } } });
write({
  schema_version: "1.0.0", host_kind: bootstrap.host_kind, host_generation: bootstrap.host_generation,
  process_id: process.pid, runtime: bootstrap.runtime, supported_methods: bootstrap.supported_methods,
});
const fallback = setTimeout(() => process.exit(88), 5000);
let mode;
let cancellations = 0;
readline.createInterface({ input: process.stdin, crlfDelay: Infinity }).on("line", (line) => {
  const frame = JSON.parse(line);
  if (frame.response) return;
  const { request, request_id: id } = frame.envelope;
  if (mode === "cancel-hang") {
    if (request.method === "request_cancel") { cancellations++; return; }
    if (request.method === "capability_invoke") {
      write({ ...ack(id), response: { outcome: "success", value: { kind: "value", payload: { cancellations } } } });
      return;
    }
  }
  if (request.method === "host_shutdown") {
    if (mode === "invalid-stop") write({ ...ack(id), response: { outcome: "success", value: { kind: "value", payload: null } } });
    else if (mode === "shutdown-rejected") write({ ...binding, request_id: id, response: { outcome: "failure", value: { code: "FIXTURE", message: "fixture-wire-secret", retryable: false } } });
    else { write(ack(id)); clearTimeout(fallback); }
    return;
  }
  if (request.method !== "mount_load") throw new Error("unexpected fixture request");
  mode = request.params.context.config.value.mode;
  if (mode === "invalid-response") {
    write({ ...ack(id), response: { outcome: "success", value: { kind: "value", payload: null } } });
    return;
  }
  if (mode === "unknown-response") { write(ack("fixture-wire-secret")); return; }
  if (mode === "invalid-version") { write({ ...ack(id), protocol_version: "fixture-wire-secret" }); return; }
  write(ack(id));
  if (["wrong-role", "duplicate-service", "service-timeout", "service-flood"].includes(mode)) {
    const service = {
      ...binding,
      host_kind: mode === "wrong-role" ? "candidate_test" : binding.host_kind,
      request_id: mode === "service-timeout" ? "fixture-wire-secret" : "fixture-service",
      direction: "java_script_to_host",
      request: { method: "credential_resolve", params: { mount_handle_id: request.params.context.mount_handle_id, slot_key: "fixture" } },
    };
    write(service);
    if (mode === "service-flood") {
      for (let i = 1; i < 10; i++) write({ ...service, request_id: "fixture-service-" + i });
    }
    if (mode === "duplicate-service") write(service);
  }
});
