import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawn } from "node:child_process";
import readline from "node:readline";

const directory = dirname(fileURLToPath(import.meta.url));
const { mode } = JSON.parse(readFileSync(join(directory, "mode.json"), "utf8"));
const bootstrap = JSON.parse(Buffer.from(process.argv[2], "hex").toString("utf8"));
const write = (value) => process.stdout.write(JSON.stringify(value) + "\n");
const binding = {
  protocol_version: bootstrap.runtime.javascript_host_protocol_version,
  host_kind: bootstrap.host_kind,
  host_generation: bootstrap.host_generation,
};
// Test failures cannot leave a permanently hanging fixture behind.
const fallback = setTimeout(() => process.exit(87), 5000);
if (mode === "cancel") {
  const child = spawn(process.execPath, ["-e", "setTimeout(() => {}, 5000)"], { stdio: "inherit" });
  writeFileSync(join(directory, "started.json"), JSON.stringify([process.pid, child.pid]));
} else {
  if (mode === "noisy") {
    await new Promise((resolve, reject) => process.stderr.write("x".repeat(2 * 1024 * 1024), (error) => error ? reject(error) : resolve()));
  }
  if (mode === "secret") {
    await new Promise((resolve) => process.stderr.write("fixture-startup-secret\n", resolve));
    write({ secret: "fixture-hello-secret" });
  } else if (mode === "invalid") {
    write("fixture-hello-secret");
  } else if (mode === "timeout") {
    // No Hello; the supervisor must apply its own deadline.
  } else {
    const hello = {
      schema_version: "1.0.0",
      host_kind: bootstrap.host_kind,
      host_generation: bootstrap.host_generation,
      process_id: process.pid,
      runtime: bootstrap.runtime,
      supported_methods: bootstrap.supported_methods,
    };
    if (mode === "wrong-generation") hello.host_generation += 1;
    if (mode === "wrong-process") hello.process_id += 1;
    if (mode === "wrong-role") hello.host_kind = "candidate_test";
    if (mode === "wrong-runtime") hello.runtime = { ...hello.runtime, node_version: "0.0.0" };
    if (mode === "wrong-contract") hello.schema_version = "0.0.0";
    write(hello);
    readline.createInterface({ input: process.stdin, crlfDelay: Infinity }).on("line", (line) => {
      const { envelope } = JSON.parse(line);
      write({ ...binding, request_id: envelope.request_id, response: { outcome: "success", value: { kind: "ack" } } });
      if (envelope.request.method === "host_shutdown") { clearTimeout(fallback); }
    });
  }
}
