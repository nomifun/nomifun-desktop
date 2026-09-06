let releaseCount = 0;

export async function activate() {
  return {
    capabilities: {
      "fixture.tool.contribution": {
        async invoke({ actionId, input, contribution }) {
          if (actionId === "fixture.reject") {
            throw new Error("fixture rejection");
          }
          if (actionId === "fixture.release_count") {
            return { releaseCount };
          }
          return {
            actionId,
            input,
            contributionId: contribution.contribution_id,
          };
        },
      },
      "fixture.context.contribution": {
        async contributeContext({ schemaRef, contribution }) {
          return {
            schemaRef,
            contributionId: contribution.contribution_id,
          };
        },
      },
      "fixture.resource.contribution": {
        async acquireResource({ bindingId, resourceKind, parameters }) {
          return {
            handleId: `fixture:${bindingId}`,
            release() {
              releaseCount += 1;
            },
            resourceKind,
            parameters,
          };
        },
      },
    },
  };
}
