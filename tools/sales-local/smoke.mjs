const nomifunUrl = process.env.NOMIFUN_SALES_URL || "http://127.0.0.1:8787";
const mockUrl = process.env.NOMIFUN_MOCK_CONTACT_URL || "http://127.0.0.1:8088";
const browserTarget =
  process.env.NOMIFUN_MOCK_CONTACT_BROWSER_URL || "http://mock-contact-site:8080";
const token =
  process.env.NOMIFUN_ACCESS_TOKEN ||
  "4eb2616de4f7f5e9aa23dded0f8d38b24ccbdf54da0344e29bd9cd19a63b7391";

let requestId = 0;
let sessionId;

function fail(message, details) {
  if (details !== undefined) console.error(details);
  throw new Error(message);
}

async function waitFor(url, label, attempts = 90) {
  for (let attempt = 1; attempt <= attempts; attempt += 1) {
    try {
      const response = await fetch(url);
      if (response.ok) return;
    } catch {}
    if (attempt === attempts) fail(`${label} did not become ready: ${url}`);
    await new Promise((resolve) => setTimeout(resolve, 1000));
  }
}

function parseEventStream(text) {
  const events = text
    .split(/\r?\n\r?\n/)
    .flatMap((block) =>
      block
        .split(/\r?\n/)
        .filter((line) => line.startsWith("data:"))
        .map((line) => line.slice(5).trim()),
    )
    .filter(Boolean)
    .map((data) => JSON.parse(data));
  return events.at(-1);
}

async function rpc(method, params, { notification = false } = {}) {
  const payload = { jsonrpc: "2.0", method };
  if (!notification) payload.id = ++requestId;
  if (params !== undefined) payload.params = params;

  const headers = {
    authorization: `Bearer ${token}`,
    accept: "application/json, text/event-stream",
    "content-type": "application/json",
  };
  if (sessionId) headers["mcp-session-id"] = sessionId;

  const response = await fetch(`${nomifunUrl}/mcp-agent`, {
    method: "POST",
    headers,
    body: JSON.stringify(payload),
  });
  sessionId ||= response.headers.get("mcp-session-id") || undefined;
  const body = await response.text();
  if (!response.ok) fail(`MCP ${method} failed with HTTP ${response.status}`, body);
  if (notification || !body.trim()) return undefined;
  const value = response.headers.get("content-type")?.includes("text/event-stream")
    ? parseEventStream(body)
    : JSON.parse(body);
  if (value?.error) fail(`MCP ${method} returned an error`, value.error);
  return value?.result;
}

function expandJsonStrings(value) {
  if (Array.isArray(value)) return value.map(expandJsonStrings);
  if (value && typeof value === "object") {
    return Object.fromEntries(
      Object.entries(value).map(([key, child]) => [key, expandJsonStrings(child)]),
    );
  }
  if (typeof value === "string") {
    const trimmed = value.trim();
    if (trimmed.startsWith("{") || trimmed.startsWith("[")) {
      try {
        return { text: value, parsed: expandJsonStrings(JSON.parse(trimmed)) };
      } catch {}
    }
  }
  return value;
}

function allStrings(value, output = []) {
  if (typeof value === "string") output.push(value);
  else if (Array.isArray(value)) value.forEach((item) => allStrings(item, output));
  else if (value && typeof value === "object") {
    Object.values(value).forEach((item) => allStrings(item, output));
  }
  return output;
}

function findKey(value, wanted) {
  if (Array.isArray(value)) {
    for (const item of value) {
      const found = findKey(item, wanted);
      if (found !== undefined) return found;
    }
  } else if (value && typeof value === "object") {
    if (Object.hasOwn(value, wanted)) return value[wanted];
    for (const item of Object.values(value)) {
      const found = findKey(item, wanted);
      if (found !== undefined) return found;
    }
  }
  return undefined;
}

async function callTool(name, args = {}, { allowError = false } = {}) {
  const result = await rpc("tools/call", { name, arguments: args });
  if (result?.isError && !allowError) fail(`${name} reported an MCP tool error`, result);
  return expandJsonStrings(result);
}

function refFor(observation, accessibleName) {
  const wanted = accessibleName.toLocaleLowerCase();

  function findEntry(value) {
    if (Array.isArray(value)) {
      for (const item of value) {
        const found = findEntry(item);
        if (found) return found;
      }
    } else if (value && typeof value === "object") {
      if (
        typeof value.ref === "string" &&
        typeof value.name === "string" &&
        value.name.toLocaleLowerCase() === wanted
      ) {
        return value.ref;
      }
      for (const item of Object.values(value)) {
        const found = findEntry(item);
        if (found) return found;
      }
    }
    return undefined;
  }

  const entryRef = findEntry(observation);
  if (entryRef) return entryRef;

  const escaped = accessibleName.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
  const patterns = [
    new RegExp(`[^\\n]*${escaped}[^\\n]*\\[ref=(f\\d+e\\d+)\\]`, "i"),
    new RegExp(`[^\\n]*\\[ref=(f\\d+e\\d+)\\][^\\n]*${escaped}`, "i"),
  ];
  for (const value of allStrings(observation)) {
    for (const pattern of patterns) {
      const match = value.match(pattern);
      if (match) return match[1];
    }
  }
  fail(`Could not find browser ref for “${accessibleName}”`, observation);
}

async function observe() {
  return callTool("nomi_browser_observe", { lane: "phase1-contact" });
}

async function typeInto(label, text) {
  const observation = await observe();
  if (process.env.NOMIFUN_SMOKE_DEBUG) {
    console.error(JSON.stringify({ label, observation }, null, 2));
  }
  await callTool("nomi_browser_act", {
    action: "type",
    lane: "phase1-contact",
    ref: refFor(observation, label),
    text,
  });
}

async function click(label) {
  const observation = await observe();
  return callTool("nomi_browser_act", {
    action: "click",
    lane: "phase1-contact",
    ref: refFor(observation, label),
  });
}

function containsText(value, expected) {
  const wanted = expected.toLocaleLowerCase();
  return allStrings(value).some((item) => item.toLocaleLowerCase().includes(wanted));
}

async function main() {
  console.log("[1/8] Waiting for the local services…");
  await Promise.all([
    waitFor(`${nomifunUrl}/`, "NomiFun"),
    waitFor(`${mockUrl}/health`, "mock contact site"),
  ]);
  await fetch(`${mockUrl}/api/submissions`, { method: "DELETE" });

  console.log("[2/8] Starting an authenticated MCP session…");
  await rpc("initialize", {
    protocolVersion: "2025-06-18",
    capabilities: {},
    clientInfo: { name: "nomifun-sales-phase1-smoke", version: "1.0.0" },
  });
  if (!sessionId) fail("NomiFun did not return an MCP session id");
  await rpc("notifications/initialized", undefined, { notification: true });

  const catalog = await rpc("tools/list", {});
  const names = new Set((catalog?.tools || []).map((tool) => tool.name));
  for (const required of [
    "nomi_browser_open",
    "nomi_browser_navigate",
    "nomi_browser_observe",
    "nomi_browser_act",
    "nomi_browser_confirm",
  ]) {
    if (!names.has(required)) fail(`Browser feature is missing MCP tool ${required}`);
  }

  console.log("[3/8] Launching containerized Chromium and opening the test site…");
  await callTool("nomi_browser_open", { lane_name: "phase1-contact" });
  await callTool("nomi_browser_navigate", {
    lane: "phase1-contact",
    url: browserTarget,
  });

  console.log("[4/8] Filling the contact form through aria refs…");
  await typeInto("Full name", "Phase One Agent");
  await typeInto("Work email", "phase1@example.test");
  await typeInto("Company", "NomiFun Local Lab");
  await typeInto(
    "How can we help?",
    "Please schedule a demo for our local browser automation evaluation.",
  );
  await click("I agree that Northstar Labs may contact me about this request.");

  console.log("[5/8] Submitting through NomiFun's out-of-band approval gate…");
  const submitResult = await click("Send inquiry");
  const callId = findKey(submitResult, "call_id");
  if (!callId) fail("The irreversible submit action did not request confirmation", submitResult);
  const confirmation = await callTool(
    "nomi_browser_confirm",
    { call_id: callId, option: "proceed_once" },
    { allowError: true },
  );
  if (confirmation?.isError) {
    console.warn(
      "The confirmation transport was ambiguous; checking the live page without repeating Submit…",
    );
  }

  console.log("[6/8] Verifying the live page shows an unambiguous success state…");
  let successObservation;
  for (let attempt = 0; attempt < 20; attempt += 1) {
    successObservation = await observe();
    if (containsText(successObservation, "Inquiry received")) break;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  if (!containsText(successObservation, "Inquiry received")) {
    fail("The form submission did not produce a visible success state", successObservation);
  }

  console.log("[7/8] Verifying the mock server recorded the browser submission…");
  let submissions;
  for (let attempt = 0; attempt < 20; attempt += 1) {
    const response = await fetch(`${mockUrl}/api/submissions`);
    submissions = await response.json();
    if (submissions.count === 1) break;
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
  const submission = submissions?.submissions?.[0];
  if (
    submissions?.count !== 1 ||
    submission?.email !== "phase1@example.test" ||
    submission?.company !== "NomiFun Local Lab"
  ) {
    fail("Recorded contact submission did not match the browser input", submissions);
  }

  console.log("[8/8] Closing the managed browser lane…");
  await callTool("nomi_browser_close_all", {});
  console.log(`PASS: browser submitted contact record ${submission.id}`);
}

try {
  await main();
} finally {
  if (sessionId) {
    await fetch(`${nomifunUrl}/mcp-agent`, {
      method: "DELETE",
      headers: { authorization: `Bearer ${token}`, "mcp-session-id": sessionId },
    }).catch(() => {});
  }
}
