const bootstrap = JSON.parse(
  Buffer.from(process.argv[2], "hex").toString("utf8"),
);
const write = (value) =>
  process.stdout.write(`${JSON.stringify(value)}\n`);

write({
  schema_version: "1.0.0",
  host_kind: bootstrap.host_kind,
  host_generation: bootstrap.host_generation,
  process_id: process.pid,
  runtime: bootstrap.runtime,
  supported_methods: bootstrap.supported_methods,
});

let buffered = "";
process.stdin.setEncoding("utf8");
process.stdin.on("data", (chunk) => {
  buffered += chunk;
  const newline = buffered.indexOf("\n");
  if (newline < 0) return;
  const frame = JSON.parse(buffered.slice(0, newline));
  write({
    protocol_version:
      bootstrap.runtime.javascript_host_protocol_version,
    host_kind: bootstrap.host_kind,
    host_generation: bootstrap.host_generation - 1,
    request_id: frame.envelope.request_id,
    response: {
      outcome: "success",
      value: { kind: "ack" },
    },
  });
});
