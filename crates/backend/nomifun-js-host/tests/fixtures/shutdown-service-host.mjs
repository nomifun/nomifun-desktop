// Deterministically issue a service request after receiving HostShutdown but
// before acknowledging it. The regular Host's immediate Ack hides this race.
import readline from "node:readline";

const bootstrap = JSON.parse(Buffer.from(process.argv[2], "hex").toString("utf8"));
const write = (value) => process.stdout.write(JSON.stringify(value) + "\n");
const binding = {
  protocol_version: bootstrap.runtime.javascript_host_protocol_version,
  host_kind: bootstrap.host_kind,
  host_generation: bootstrap.host_generation,
};
const ack = (requestId) => write({
  ...binding,
  request_id: requestId,
  response: { outcome: "success", value: { kind: "ack" } },
});

write({
  schema_version: "1.0.0",
  host_kind: bootstrap.host_kind,
  host_generation: bootstrap.host_generation,
  process_id: process.pid,
  runtime: bootstrap.runtime,
  supported_methods: bootstrap.supported_methods,
});

let mountHandle;
let shutdownId;
readline.createInterface({ input: process.stdin, crlfDelay: Infinity }).on("line", (line) => {
  const frame = JSON.parse(line);
  if (frame.response) {
    if (frame.response.outcome !== "failure") throw new Error("shutdown admitted a new service");
    ack(shutdownId);
    return;
  }
  const { request, request_id: requestId } = frame.envelope;
  if (request.method === "mount_load") {
    mountHandle = request.params.context.mount_handle_id;
    if (request.params.context.config.value.unknownServiceHandle) {
      write({
        ...binding,
        request_id: "unknown-activation-service",
        direction: "java_script_to_host",
        request: { method: "credential_resolve", params: { mount_handle_id: "not-reserved", slot_key: "fixture" } },
      });
      return;
    }
    ack(requestId);
  } else if (request.method === "host_shutdown") {
    shutdownId = requestId;
    write({
      ...binding,
      request_id: "shutdown-service",
      direction: "java_script_to_host",
      request: { method: "credential_resolve", params: { mount_handle_id: mountHandle, slot_key: "fixture" } },
    });
  }
});
