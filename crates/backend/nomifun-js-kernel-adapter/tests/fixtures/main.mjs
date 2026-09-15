let releaseCount = 0;
let contextCancelCount = 0;

export async function activate() {
  const resources = new Map();
  return {
    capabilities: {
      "fixture.tool.contribution": {
        async invoke({ actionId, input, contribution, dependencies }) {
          if (input?.dependency) {
            try { return await dependencies.invoke(input.dependency); }
            catch (error) { return { dependencyError: error.message }; }
          }
          if (actionId === "fixture.reject") {
            throw new Error("fixture rejection");
          }
          if (actionId === "fixture.release_count") {
            return { releaseCount, contextCancelCount };
          }
          return {
            actionId,
            input,
            contributionId: contribution.contribution_id,
            resourceBindings: [...resources.values()],
          };
        },
      },
      "fixture.ui.contribution": {
        async invoke({ actionId, input, contribution, dependencies }) {
          if (input?.dependency) return await dependencies.invoke(input.dependency);
          return {
            actionId,
            input,
            contributionId: contribution.contribution_id,
          };
        },
      },
      "fixture.grandchild.contribution": {
        async invoke({ actionId, input, contribution }) {
          return { actionId, input, contributionId: contribution.contribution_id };
        },
      },
      "fixture.context.contribution": {
        async contributeContext({ schemaRef, contribution, input, signal, dependencies }) {
          if (input?.turn?.text?.startsWith('{"dependency":')) {
            try { return await dependencies.invoke(JSON.parse(input.turn.text).dependency); }
            catch (error) { return { dependencyError: error.message }; }
          }
          if (input?.turn?.text === "wait-for-cancel") {
            await new Promise((resolve) => {
              const aborted = () => { contextCancelCount += 1; resolve(); };
              if (signal.aborted) aborted();
              else signal.addEventListener("abort", aborted, { once: true });
            });
          }
          return {
            schemaRef,
            input,
            contributionId: contribution.contribution_id,
            resourceBindings: [...resources.values()],
          };
        },
      },
      "fixture.resource.contribution": {
        async acquireResource({ bindingId, resourceKind, parameters }) {
          resources.set(bindingId, { bindingId, resourceKind, parameters });
          return {
            handleId: `fixture:${bindingId}`,
            release() {
              releaseCount += 1;
              resources.delete(bindingId);
            },
            resourceKind,
            parameters,
          };
        },
      },
    },
  };
}
