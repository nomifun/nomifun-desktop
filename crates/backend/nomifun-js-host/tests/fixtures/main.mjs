import { appendFileSync } from "node:fs";
import { spawn } from "node:child_process";

export async function activate({ mount }) {
  const contributionId = `contribution.${mount.target.mount_id}`;
  return {
    capabilities: {
      [contributionId]: {
        async invoke({ actionId, input, signal, contribution }) {
          switch (actionId) {
            case "echo":
              return {
                input,
                mount_id: mount.target.mount_id,
                artifact_digest: contribution.target.artifact_digest,
              };
            case "reject":
              throw new Error("fixture rejection");
            case "hang":
              await new Promise(() => {});
              return null;
            case "wait_for_cancel":
              await new Promise((resolve, reject) => {
                signal.addEventListener(
                  "abort",
                  () => {
                    const error = new Error("fixture canceled");
                    error.name = "AbortError";
                    reject(error);
                  },
                  { once: true },
                );
              });
              return null;
            case "crash":
              process.exit(81);
              return null;
            case "child_then_crash": {
              const heartbeat = input.heartbeat;
              spawn(
                process.execPath,
                [
                  "-e",
                  `const fs=require("node:fs");setInterval(()=>fs.appendFileSync(${JSON.stringify(
                    heartbeat,
                  )},"x"),20)`,
                ],
                { stdio: "ignore" },
              );
              appendFileSync(heartbeat, "root");
              setTimeout(() => process.exit(82), 80);
              await new Promise(() => {});
              return null;
            }
            default:
              throw new Error(`unknown fixture action ${actionId}`);
          }
        },
      },
    },
  };
}
