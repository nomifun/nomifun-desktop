// A valid peer that stops reading after MountLoad. The fallback exit bounds
// regression failures; passing tests must reap it well before this deadline.
import readline from "node:readline";

const bootstrap = JSON.parse(Buffer.from(process.argv[2], "hex").toString("utf8"));
const write = (value) => process.stdout.write(JSON.stringify(value) + "\n");
const binding = {
  protocol_version: bootstrap.runtime.javascript_host_protocol_version,
  host_kind: bootstrap.host_kind,
  host_generation: bootstrap.host_generation,
};
write({
  schema_version: "1.0.0",
  host_kind: bootstrap.host_kind,
  host_generation: bootstrap.host_generation,
  process_id: process.pid,
  runtime: bootstrap.runtime,
  supported_methods: bootstrap.supported_methods,
});
const input = readline.createInterface({ input: process.stdin, crlfDelay: Infinity });
input.on("line", (line) => {
  const { request, request_id: requestId } = JSON.parse(line).envelope;
  if (request.method !== "mount_load") throw new Error("unexpected request before pause");
  input.pause();
  setTimeout(() => process.exit(86), 4000);
  write({ ...binding, request_id: requestId, response: { outcome: "success", value: { kind: "ack" } } });
  if (request.params.context.config.value.service) {
    write({
      ...binding,
      request_id: "backpressure-service",
      direction: "java_script_to_host",
      request: { method: "credential_resolve", params: {
        mount_handle_id: request.params.context.mount_handle_id, slot_key: "fixture",
      } },
    });
  }
});
