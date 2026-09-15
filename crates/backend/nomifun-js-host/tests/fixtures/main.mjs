import { appendFileSync } from "node:fs";
import { spawn } from "node:child_process";

let resourceReleaseCount = 0;
let retiredSdk;
let retiredDependencies;

export async function activate({ mount, sdk }) {
  const cancellationState = { tool_started: 0, tool_cancelled: 0, context_started: 0, context_cancelled: 0 };
  async function waitForCancellation(signal, kind) {
    cancellationState[`${kind}_started`] += 1;
    await new Promise((resolve, reject) => {
      const abort = () => {
        cancellationState[`${kind}_cancelled`] += 1;
        const error = new Error("fixture canceled");
        error.name = "AbortError";
        reject(error);
      };
      if (signal.aborted) abort();
      else signal.addEventListener("abort", abort, { once: true });
    });
  }
  let serviceResult;
  if (mount.config.value.activation === "service") {
    serviceResult = await sdk.credential.resolve("activation");
  }
  if (mount.config.value.activation === "service_then_reject") {
    retiredSdk = sdk;
    void sdk.credential.resolve("activation").catch(() => {});
    throw new Error("fixture activation rejected with an outstanding service");
  }
  if (mount.config.value.activation === "reject") {
    throw new Error("fixture activation rejected");
  }
  if (mount.config.value.activation === "hang") {
    await new Promise(() => {});
  }
  const contributionId = `contribution.${mount.target.mount_id}`;
  return {
    async deactivate() {
      retiredSdk = sdk;
      if (mount.config.value.deactivationService) {
        await sdk.credential.resolve("deactivation");
      }
      if (mount.config.value.deactivationDetachedService) {
        void sdk.credential.resolve("deactivation").catch(() => {});
      }
    },
    capabilities: {
      [contributionId]: {
        async invoke({ actionId, input, signal, contribution, dependencies }) {
          switch (actionId) {
            case "dependency":
              return await dependencies.invoke(input);
            case "retain_dependency":
              retiredDependencies = dependencies;
              if (input.reject) throw new Error("retained then rejected");
              return null;
            case "late_dependency":
              return await retiredDependencies.invoke(input).catch((error) => ({ error: error.message }));
            case "detach_dependency":
              serviceResult = dependencies.invoke(input).catch((error) => ({ error: error.message }));
              return null;
            case "start_service":
              serviceResult = sdk.credential.resolve(input.slot ?? "fixture")
                .catch((error) => ({ error: error.message }));
              return null;
            case "await_service":
              return await serviceResult;
            case "stale_service":
              return await retiredSdk.credential.resolve("retired")
                .catch((error) => ({ error: error.message }));
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
              await waitForCancellation(signal, "tool");
              return null;
            case "cancellation_state":
              return { ...cancellationState };
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
            case "resource_release_count":
              return { count: resourceReleaseCount };
            default:
              throw new Error(`unknown fixture action ${actionId}`);
          }
        },
        async contributeContext({ schemaRef, contribution, signal, input, dependencies }) {
          if (schemaRef === "schema://fixture/dependencies") {
            const command = JSON.parse(input.turn.text);
            return this.invoke({ actionId: command.action, input: command.input, contribution, signal, dependencies });
          }
          if (schemaRef === "schema://fixture/wait-for-cancel") {
            await waitForCancellation(signal, "context");
          }
          return {
            schema_ref: schemaRef,
            mount_id: contribution.target.mount_id,
          };
        },
        async acquireResource({ bindingId, resourceKind, parameters }) {
          if (parameters.waitForService) await sdk.credential.resolve("acquire");
          return {
            handleId: parameters.handleId ?? `${mount.target.mount_id}:${bindingId}`,
            async release() {
              if (parameters.requireReceiver && this.handleId !== parameters.handleId) {
                throw new Error("release callback lost its receiver");
              }
              resourceReleaseCount += 1;
              if (parameters.releaseLog) appendFileSync(parameters.releaseLog, bindingId + "\n");
              if (parameters.waitForReleaseService) await sdk.credential.resolve("release");
              if (parameters.failRelease) throw new Error("fixture release failed");
            },
            resourceKind,
            parameters,
          };
        },
      },
    },
  };
}
