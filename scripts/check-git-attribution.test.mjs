import { describe, expect, test } from "bun:test";

import {
  findIdentityViolations,
  findMessageViolations,
  validateCommitData,
} from "./check-git-attribution.mjs";

describe("Git attribution policy", () => {
  test("accepts human author and committer identities", () => {
    expect(
      findIdentityViolations({
        author: "RiKa0-0 <2206491416@qq.com>",
        committer: "muri <2206491416@qq.com>",
      }),
    ).toEqual([]);
  });

  test("rejects model and vendor identities", () => {
    expect(
      findIdentityViolations({
        author: "Codex <codex@openai.com>",
        committer: "Human <human@example.com>",
      }),
    ).toHaveLength(1);
  });

  test("rejects AI co-author trailers", () => {
    expect(
      findMessageViolations(
        "fix: example\n\nCo-Authored-By: Claude Opus <noreply@anthropic.com>\n",
      ),
    ).toHaveLength(1);
  });

  test("rejects other AI credit trailers", () => {
    expect(findMessageViolations("Generated-by: GPT-5\nAssisted-by: Gemini\n")).toHaveLength(2);
  });

  test("allows human co-author trailers", () => {
    expect(
      findMessageViolations("Co-authored-by: Example Developer <dev@example.com>\n"),
    ).toEqual([]);
  });

  test("allows technical AI product references in normal prose", () => {
    expect(
      findMessageViolations(
        "fix(agent): handle Claude and Codex session recovery\n\nKeep provider behavior aligned.\n",
      ),
    ).toEqual([]);
  });

  test("validates complete commit metadata", () => {
    expect(
      validateCommitData({
        authorName: "Developer",
        authorEmail: "developer@example.com",
        committerName: "Developer",
        committerEmail: "developer@example.com",
        message: "Generated-by: OpenAI\n",
      }),
    ).toHaveLength(1);
  });
});
