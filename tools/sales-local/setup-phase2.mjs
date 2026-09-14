try {
  process.loadEnvFile?.();
} catch {}

const baseUrl = process.env.NOMIFUN_SALES_URL || "http://127.0.0.1:8787";
const username = process.env.NOMIFUN_ADMIN_USERNAME || "admin";
const password = process.env.NOMIFUN_ADMIN_PASSWORD || "phase1-local-admin-only";

const presetName = "本地销售联络助手";
const skillName = "sales-contact-operator";

function fail(message, details) {
  if (details !== undefined) console.error(details);
  throw new Error(message);
}

async function responseJson(response, context) {
  const text = await response.text();
  let body;
  try {
    body = text ? JSON.parse(text) : {};
  } catch {
    fail(`${context} returned invalid JSON`, text.slice(0, 500));
  }
  if (!response.ok || body.success === false) {
    fail(`${context} failed with HTTP ${response.status}`, body);
  }
  return body;
}

function csrfFrom(response) {
  const cookies = response.headers.getSetCookie?.() || [response.headers.get("set-cookie") || ""];
  for (const cookie of cookies) {
    const match = cookie.match(/(?:^|;\s*)nomifun-csrf-token=([^;]+)/);
    if (match) return match[1];
  }
  return undefined;
}

async function login() {
  const statusResponse = await fetch(`${baseUrl}/api/auth/status`);
  if (!statusResponse.ok) fail("Could not reach NomiFun authentication status");
  const csrf = csrfFrom(statusResponse);
  if (!csrf) fail("NomiFun did not issue a CSRF token");

  const loginResponse = await fetch(`${baseUrl}/login`, {
    method: "POST",
    headers: { "content-type": "application/json" },
    body: JSON.stringify({ username, password }),
  });
  const loginBody = await responseJson(loginResponse, "NomiFun login");
  if (!loginBody.token) fail("NomiFun login did not return a session token");
  return { token: loginBody.token, csrf };
}

async function main() {
  console.log("[1/5] Authenticating with the local NomiFun instance…");
  const { token, csrf } = await login();

  async function api(path, { method = "GET", body } = {}) {
    const response = await fetch(`${baseUrl}${path}`, {
      method,
      headers: {
        authorization: `Bearer ${token}`,
        "x-csrf-token": csrf,
        cookie: `nomifun-csrf-token=${csrf}`,
        ...(body === undefined ? {} : { "content-type": "application/json" }),
      },
      body: body === undefined ? undefined : JSON.stringify(body),
    });
    const payload = await responseJson(response, `${method} ${path}`);
    return payload.data;
  }

  console.log("[2/5] Verifying the repository-managed sales skill…");
  const skills = await api("/api/skills");
  const skill = skills.find((item) => item.name === skillName);
  if (!skill) {
    fail(
      `Skill ${skillName} is not mounted. Recreate the Compose service before retrying.`,
    );
  }

  console.log("[3/5] Checking model readiness…");
  const [providers, freeStatus] = await Promise.all([
    api("/api/providers"),
    api("/api/model-services/free/status").catch(() => undefined),
  ]);
  const chatModels = providers.flatMap((provider) =>
    provider.enabled
      ? provider.models
          .filter(
            (model) =>
              model.enabled &&
              model.capabilities.some((capability) => capability.task === "chat"),
          )
          .map((model) => ({
            providerId: provider.provider_id,
            provider: provider.name,
            model: model.model,
          }))
      : [],
  );
  const preferredModelNames = [
    "deepseek-v4-flash-free",
    "mimo-v2.5-free",
    "big-pickle",
  ];
  const suggestedModel =
    preferredModelNames
      .map((name) => chatModels.find((item) => item.model === name))
      .find(Boolean) || chatModels[0];

  console.log("[4/5] Creating or refreshing the sales preset…");
  const presetBody = {
    name: presetName,
    description: "研究目标公司、准备诚实的联络内容，并在明确授权时自动提交合格表单。",
    routing_description:
      "适用于有界目标市场的公开资料研究、适配判断、联络表单自动提交和本地安全演练。",
    instructions:
      "使用 sales-contact-operator 技能。先用 browser_crawl_many 的 urls 数组做只读发现和官网核实，确认官方表单后才打开一个交互 Lane。逐家公司处理，严格限制浏览器恢复次数；任务明确开启自动提交时，只对合格公司提交一次。候选进入研究和到达终态时都使用同一 event_id 输出 SALES_DASHBOARD_EVENT。不要输出内部推理或猜测的 URL。默认先使用本地模拟 Contact Form 演练。",
    fallback_allowed: true,
    targets: ["conversation"],
    included_skills: [{ skill_name: skillName, required: true }],
    examples: [
      "请在本地模拟 Contact Form 上演练一次自动销售联络流程，核实后直接提交并输出 Dashboard 事件。",
      "研究这个明确指定的公司官网，判断是否符合我的 ICP，并列出证据；暂时不要联系。",
    ],
    name_i18n: { "en-US": "Local Sales Contact Assistant" },
    description_i18n: {
      "en-US":
        "Research targets, prepare honest outreach, and auto-submit qualified forms when explicitly authorized.",
    },
    instructions_i18n: {
      "en-US":
        "Use the sales-contact-operator skill. Discover with browser_crawl_many and its plural urls array before opening one interactive Lane for a verified official form. Process targets sequentially, honor the strict recovery budget, and auto-submit each qualified form once only when explicitly authorized. Emit researching and terminal SALES_DASHBOARD_EVENT lines with the same event_id. Do not expose private reasoning or guess URLs. Rehearse on the local mock Contact Form first.",
    },
  };

  const presets = await api("/api/presets");
  const matches = presets.filter(
    (preset) => preset.source === "user" && preset.name === presetName,
  );
  if (matches.length > 1) {
    fail(`Found ${matches.length} user presets named ${presetName}; remove duplicates first.`);
  }

  const existingModelPreferences = matches[0]?.model_preferences || [];
  const hasEnabledModelPreference = existingModelPreferences.some((preference) =>
    chatModels.some(
      (model) =>
        model.providerId === preference.provider_id && model.model === preference.model,
    ),
  );
  if (!hasEnabledModelPreference && suggestedModel) {
    presetBody.model_preferences = [
      {
        provider_id: suggestedModel.providerId,
        model: suggestedModel.model,
        required: false,
      },
    ];
  }

  let preset;
  if (matches.length === 1) {
    preset = await api(`/api/presets/${encodeURIComponent(matches[0].preset_id)}`, {
      method: "PUT",
      body: presetBody,
    });
  } else {
    preset = await api("/api/presets", { method: "POST", body: presetBody });
  }

  console.log("[5/5] Resolving the preset exactly as a conversation will use it…");
  const resolved = await api(
    `/api/presets/${encodeURIComponent(preset.preset_id)}/resolve`,
    {
      method: "POST",
      body: { target: "conversation", locale: "zh-CN", overrides: {} },
    },
  );
  if (!resolved.included_skills.includes(skillName)) {
    fail(`Resolved preset does not include required skill ${skillName}`, resolved);
  }
  if (!resolved.resolved_agent_id) {
    fail("Resolved preset did not select an enabled Agent", resolved);
  }
  if (suggestedModel && !resolved.resolved_model) {
    fail("Resolved preset did not select an enabled chat model", resolved);
  }

  console.log(
    `PASS: preset “${preset.name}” resolves an Agent, ${skillName}, and ${
      resolved.resolved_model?.model || "the model selected in WebUI"
    }`,
  );
  if (chatModels.length > 0) {
    console.log(`READY: ${chatModels.length} enabled chat model(s) are available.`);
    for (const item of chatModels.slice(0, 5)) {
      console.log(`- ${item.provider}: ${item.model}`);
    }
  } else {
    console.log("ACTION REQUIRED: configure and enable one chat model in the WebUI Model Hub.");
  }
  if (freeStatus) {
    console.log(
      `Managed free models: ${freeStatus.enabled ? "enabled" : "disabled"}, ${freeStatus.availability}.`,
    );
  }
}

await main();
