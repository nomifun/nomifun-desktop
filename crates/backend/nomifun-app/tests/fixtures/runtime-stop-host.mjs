import { existsSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import readline from "node:readline";

const directory = dirname(fileURLToPath(import.meta.url));
const bootstrap = JSON.parse(Buffer.from(process.argv[2], "hex").toString("utf8"));
const write = value => process.stdout.write(JSON.stringify(value) + "\n");
write({ schema_version: "1.0.0", host_kind: bootstrap.host_kind,
  host_generation: bootstrap.host_generation, process_id: process.pid,
  runtime: bootstrap.runtime, supported_methods: bootstrap.supported_methods });
const fallback = setTimeout(() => process.exit(89), 10000);
readline.createInterface({ input: process.stdin, crlfDelay: Infinity }).on("line", async line => {
  const { envelope } = JSON.parse(line);
  if (envelope.request.method === "host_shutdown") {
    writeFileSync(join(directory, "stopping"), "ready");
    while (!existsSync(join(directory, "finish"))) await new Promise(resolve => setTimeout(resolve, 10));
    clearTimeout(fallback);
  }
  write({ protocol_version: bootstrap.runtime.javascript_host_protocol_version,
    host_kind: bootstrap.host_kind, host_generation: bootstrap.host_generation,
    request_id: envelope.request_id, response: { outcome: "success", value: { kind: "ack" } } });
});
