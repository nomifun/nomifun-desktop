import { appendFile, mkdir, readFile, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { fileURLToPath } from "node:url";
import { dirname, join } from "node:path";
import { randomUUID } from "node:crypto";

const appDir = dirname(fileURLToPath(import.meta.url));
const html = await readFile(join(appDir, "index.html"), "utf8");
const dataDir = process.env.MOCK_CONTACT_DATA_DIR || "/data";
const submissionsPath = join(dataDir, "submissions.jsonl");
const port = Number(process.env.PORT || 8080);

await mkdir(dataDir, { recursive: true });

function json(response, status, body) {
  const payload = JSON.stringify(body);
  response.writeHead(status, {
    "content-type": "application/json; charset=utf-8",
    "content-length": Buffer.byteLength(payload),
    "cache-control": "no-store",
    "x-content-type-options": "nosniff",
  });
  response.end(payload);
}

function successPage(response, submissionId) {
  const payload = `<!doctype html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <meta name="robots" content="noindex, nofollow" />
    <title>Inquiry received — Northstar Labs</title>
  </head>
  <body>
    <main>
      <h1>Inquiry received</h1>
      <p>This local simulation sent no email.</p>
      <p>Reference: <code>${submissionId}</code></p>
    </main>
  </body>
</html>`;
  response.writeHead(201, {
    "content-type": "text/html; charset=utf-8",
    "content-length": Buffer.byteLength(payload),
    "cache-control": "no-store",
    "x-robots-tag": "noindex, nofollow",
    "x-content-type-options": "nosniff",
  });
  response.end(payload);
}

async function readBody(request) {
  const chunks = [];
  let size = 0;
  for await (const chunk of request) {
    size += chunk.length;
    if (size > 64 * 1024) {
      throw new Error("request body is too large");
    }
    chunks.push(chunk);
  }
  return Buffer.concat(chunks).toString("utf8");
}

async function listSubmissions() {
  try {
    const contents = await readFile(submissionsPath, "utf8");
    return contents
      .split("\n")
      .filter(Boolean)
      .map((line) => JSON.parse(line));
  } catch (error) {
    if (error.code === "ENOENT") return [];
    throw error;
  }
}

function validateContact(input) {
  const contact = {
    name: String(input.name || "").trim(),
    email: String(input.email || "").trim(),
    company: String(input.company || "").trim(),
    message: String(input.message || "").trim(),
    consent: input.consent === true || input.consent === "on",
  };
  const errors = {};
  if (contact.name.length < 2) errors.name = "Please enter your name.";
  if (!/^[^\s@]+@[^\s@]+\.[^\s@]+$/.test(contact.email)) {
    errors.email = "Please enter a valid email address.";
  }
  if (contact.message.length < 10) {
    errors.message = "Please tell us a little more about your project.";
  }
  if (!contact.consent) errors.consent = "Consent is required.";
  return { contact, errors };
}

const server = createServer(async (request, response) => {
  const url = new URL(request.url || "/", "http://localhost");
  try {
    if (request.method === "GET" && url.pathname === "/") {
      response.writeHead(200, {
        "content-type": "text/html; charset=utf-8",
        "cache-control": "no-store",
        "x-robots-tag": "noindex, nofollow",
        "x-content-type-options": "nosniff",
      });
      response.end(html);
      return;
    }

    if (request.method === "GET" && url.pathname === "/health") {
      json(response, 200, { ok: true });
      return;
    }

    if (request.method === "GET" && url.pathname === "/api/submissions") {
      const submissions = await listSubmissions();
      json(response, 200, { count: submissions.length, submissions });
      return;
    }

    if (request.method === "DELETE" && url.pathname === "/api/submissions") {
      await writeFile(submissionsPath, "", { mode: 0o600 });
      json(response, 200, { ok: true });
      return;
    }

    if (request.method === "POST" && url.pathname === "/api/contact") {
      const body = await readBody(request);
      let input;
      const contentType = request.headers["content-type"] || "";
      if (contentType.startsWith("application/x-www-form-urlencoded")) {
        input = Object.fromEntries(new URLSearchParams(body));
      } else {
        try {
          input = JSON.parse(body);
        } catch {
          json(response, 400, { error: "invalid_body" });
          return;
        }
      }
      const { contact, errors } = validateContact(input);
      if (Object.keys(errors).length > 0) {
        json(response, 422, { error: "validation_failed", fields: errors });
        return;
      }
      const submission = {
        id: randomUUID(),
        submittedAt: new Date().toISOString(),
        ...contact,
      };
      await appendFile(submissionsPath, `${JSON.stringify(submission)}\n`, {
        encoding: "utf8",
        mode: 0o600,
      });
      if (contentType.startsWith("application/x-www-form-urlencoded")) {
        successPage(response, submission.id);
      } else {
        json(response, 201, { ok: true, submissionId: submission.id });
      }
      return;
    }

    json(response, 404, { error: "not_found" });
  } catch (error) {
    console.error(error);
    json(response, error.message === "request body is too large" ? 413 : 500, {
      error: "server_error",
    });
  }
});

server.listen(port, "0.0.0.0", () => {
  console.log(`mock contact site listening on 0.0.0.0:${port}`);
});

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => server.close(() => process.exit(0)));
}
