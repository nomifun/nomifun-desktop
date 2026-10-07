// Installed Plugin Package v1 entrypoint.
let aborted = 0;
export async function activate() {
  return { capabilities: { "capability:example.discovery": {
    async invoke({ input, signal }) {
      if (input.query === "reject") throw new Error("private plugin diagnostic");
      if (input.query === "outside") return { names: ["Alpha", "NotAuthorized"] };
      if (input.query === "duplicate") return { names: ["Alpha", "Alpha"] };
      if (input.query === "wait") await new Promise(resolve => {
        const stop = () => { aborted++; resolve(); };
        if (signal.aborted) stop(); else signal.addEventListener("abort", stop, { once: true });
      });
      if (input.query === "cancelled") return { names: aborted ? ["Beta"] : [] };
      if (input.candidates.some(candidate => "input_schema" in candidate || "activation_identity" in candidate)) {
        throw new Error("host leaked schema or authority to discovery");
      }
      // Deliberately different from default alphabetical ranking, including
      // short queries. It returns only names supplied by the host.
      return { names: input.candidates.map(candidate => candidate.name).sort().reverse().slice(0, input.limit) };
    }
  } } };
}
