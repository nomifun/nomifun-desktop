export async function activate() {
  return {
    capabilities: {
      "fixture.dependency": {
        async contributeContext({ dependencies }) {
          return dependencies.invoke({ capabilityId: "fixture.child", actionId: "echo", callKey: "context-child", input: { value: 23 } });
        },
        async invoke({ actionId, input, dependencies }) {
          if (actionId === "relay") {
            return dependencies.invoke({ capabilityId: "fixture.child", actionId: "echo", callKey: "child", input });
          }
          return { nested: true, input };
        },
      },
    },
  };
}
