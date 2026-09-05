const baseUrl = process.env.NOMIFUN_SALES_URL || "http://127.0.0.1:8787";
const adminUsername = process.env.NOMIFUN_ADMIN_USERNAME || "admin";
const adminPassword =
  process.env.NOMIFUN_ADMIN_PASSWORD || "phase1-local-admin-only";
const memberUsername =
  process.env.NOMIFUN_TENANT_SMOKE_USERNAME || "sales-isolation-check";
const memberPassword =
  process.env.NOMIFUN_TENANT_SMOKE_PASSWORD || "TenantCheck-2026!";

function fail(message, details) {
  if (details !== undefined) console.error(details);
  throw new Error(message);
}

function csrfFrom(response) {
  const cookies = response.headers.getSetCookie?.() || [response.headers.get("set-cookie") || ""];
  for (const cookie of cookies) {
    const match = cookie.match(/(?:^|;\s*)nomifun-csrf-token=([^;]+)/);
    if (match) return match[1];
  }
  return undefined;
}

async function parseResponse(response, context, expectedStatus) {
  const text = await response.text();
  let body;
  try {
    body = text ? JSON.parse(text) : {};
  } catch {
    fail(`${context} returned invalid JSON`, text.slice(0, 500));
  }
  if (expectedStatus !== undefined) {
    if (response.status !== expectedStatus) {
      fail(`${context} returned HTTP ${response.status}, expected ${expectedStatus}`, body);
    }
    return body;
  }
  if (!response.ok || body.success === false) {
    fail(`${context} failed with HTTP ${response.status}`, body);
  }
  return body?.data ?? body;
}

async function login(username, password) {
  const statusResponse = await fetch(`${baseUrl}/api/auth/status`);
  if (!statusResponse.ok) fail("Could not reach NomiFun authentication status");
  const csrf = csrfFrom(statusResponse);
  if (!csrf) fail("NomiFun did not issue a CSRF token");

  const response = await fetch(`${baseUrl}/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username, password }),
  });
  const body = await parseResponse(response, `Login for ${username}`);
  if (!body.token) fail(`Login for ${username} did not return a session token`);
  return { token: body.token, csrf };
}

function client(session) {
  return async (path, { method = "GET", body, expectedStatus } = {}) => {
    const response = await fetch(`${baseUrl}${path}`, {
      method,
      headers: {
        authorization: `Bearer ${session.token}`,
        "x-csrf-token": session.csrf,
        cookie: `nomifun-csrf-token=${session.csrf}`,
        ...(body === undefined ? {} : { "content-type": "application/json" }),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    return parseResponse(response, `${method} ${path}`, expectedStatus);
  };
}

function workspace(companyName) {
  return {
    version: 1,
    companyProfile: {
      companyName,
      website: "",
      businessSummary: "Tenant isolation verification",
      valueProposition: "",
      targetCustomer: "",
      senderName: "",
      senderEmail: "",
    },
    tasks: [],
    companies: [],
    results: [],
  };
}

async function main() {
  console.log("[1/6] Logging in as the installation owner…");
  const ownerApi = client(await login(adminUsername, adminPassword));
  const ownerIdentity = await ownerApi("/api/auth/user");
  const ownerId = ownerIdentity.user?.user_id;
  if (!ownerId) fail("Owner identity did not include user_id", ownerIdentity);

  console.log("[2/6] Ensuring one non-owner sales account exists…");
  let users = (await ownerApi("/api/sales/admin/users")).users;
  let member = users.find((user) => user.username === memberUsername);
  if (!member) {
    member = (
      await ownerApi("/api/sales/admin/users", {
        method: "POST",
        body: { username: memberUsername, password: memberPassword },
      })
    ).user;
  } else {
    await ownerApi(`/api/sales/admin/users/${encodeURIComponent(member.user_id)}/password`, {
      method: "POST",
      body: { password: memberPassword },
    });
  }

  const memberApi = client(await login(memberUsername, memberPassword));
  const memberIdentity = await memberApi("/api/auth/user");
  const memberId = memberIdentity.user?.user_id;
  if (!memberId || memberId === ownerId) fail("Member identity is not independent", memberIdentity);

  console.log("[3/6] Saving different workspaces under the two authenticated users…");
  const ownerOriginal = (await ownerApi("/api/sales/workspace")).workspace;
  const memberOriginal = (await memberApi("/api/sales/workspace")).workspace;
  try {
    await ownerApi("/api/sales/workspace", {
      method: "PUT",
      body: { expected_user_id: ownerId, workspace: workspace("Isolation Owner Company") },
    });
    await memberApi("/api/sales/workspace", {
      method: "PUT",
      body: { expected_user_id: memberId, workspace: workspace("Isolation Member Company") },
    });

    console.log("[4/6] Reading each workspace back through its own login…");
    const ownerRead = (await ownerApi("/api/sales/workspace")).workspace;
    const memberRead = (await memberApi("/api/sales/workspace")).workspace;
    if (ownerRead.companyProfile.companyName !== "Isolation Owner Company") {
      fail("Owner workspace did not round-trip independently", ownerRead);
    }
    if (memberRead.companyProfile.companyName !== "Isolation Member Company") {
      fail("Member workspace did not round-trip independently", memberRead);
    }

    console.log("[5/6] Proving cross-account writes and account admin are rejected…");
    await memberApi("/api/sales/workspace", {
      method: "PUT",
      body: { expected_user_id: ownerId, workspace: workspace("Must Never Persist") },
      expectedStatus: 409,
    });
    await memberApi("/api/sales/admin/users", { expectedStatus: 403 });
    const memberAfterRejectedWrite = (await memberApi("/api/sales/workspace")).workspace;
    if (memberAfterRejectedWrite.companyProfile.companyName !== "Isolation Member Company") {
      fail("Rejected cross-account write changed member data", memberAfterRejectedWrite);
    }

    const ownerAccess = await ownerApi("/api/sales/access");
    const memberAccess = await memberApi("/api/sales/access");
    if (!ownerAccess.is_instance_owner || memberAccess.is_instance_owner) {
      fail("Owner access flags are incorrect", { ownerAccess, memberAccess });
    }

  } finally {
    console.log("[6/6] Restoring the workspaces used before this test…");
    await ownerApi("/api/sales/workspace", {
      method: "PUT",
      body: { expected_user_id: ownerId, workspace: ownerOriginal },
    });
    await memberApi("/api/sales/workspace", {
      method: "PUT",
      body: { expected_user_id: memberId, workspace: memberOriginal },
    });
  }

  console.log(
    `PASS: ${adminUsername} and ${memberUsername} have isolated sales workspaces.`,
  );
}

await main();
