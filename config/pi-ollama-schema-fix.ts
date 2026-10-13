// Pi extension: give every object tool schema a `properties` field before a request goes to Ollama.
// Ollama's llama.cpp engine (e.g. the JetBrains Qwen3.8/3.6 27B blend GGUF) rejects `{"type":"object"}`
// without it ("JSON schema error at #: properties must be an object"), and T3 Code's injected MCP tools
// (orchestrator_capabilities, ...) use exactly that shape. setup.bat copies this to ~/.pi/agent/extensions/.

function fixSchema(schema: unknown): void {
  if (Array.isArray(schema)) {
    for (const item of schema) fixSchema(item);
    return;
  }
  if (typeof schema !== "object" || schema === null) return;
  const record = schema as Record<string, unknown>;
  if (record.type === "object" && (typeof record.properties !== "object" || record.properties === null)) {
    record.properties = {};
  }
  for (const value of Object.values(record)) fixSchema(value);
}

export default function (pi: any) {
  pi.on("before_provider_request", (event: any, ctx: any) => {
    if (ctx.model?.provider !== "ollama") return;
    const tools = event.payload?.tools;
    if (!Array.isArray(tools)) return;
    for (const tool of tools) fixSchema(tool?.function?.parameters);
    return event.payload;
  });
}
