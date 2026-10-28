// Pi extension: keep T3 Code's MCP tools behind tool_search for local models (llama.cpp, Ollama).
// T3 Code's Pi bridge declares all ~85 of its tools to every openai-completions model - about 29k tokens
// of schemas before the first message. That overflows a local model's context (llama.cpp: "request exceeds
// the available context size") or leaves it a few hundred tokens to answer, so it stops calling tools.
// Here only T3's three core tools stay declared; the model loads the others with tool_search when needed.
// setup.bat copies this to ~/.pi/agent/extensions/. It loads after T3's bridge, so its handlers run later.

const PREFIX = "mcp__t3-code__";
// the ones T3 itself keeps direct for other providers
const DIRECT = new Set(["orchestrator_capabilities", "delegate_task", "task_status"].map((name) => PREFIX + name));

function isLocal(model: any): boolean {
  try {
    const host = new URL(model?.baseUrl ?? "").hostname;
    return host === "localhost" || host === "127.0.0.1" || host === "[::1]" || host === "::1";
  } catch {
    return false;
  }
}

export default function (pi: any) {
  // tools the model loaded with tool_search in this session stay declared
  const loaded = new Set<string>();

  const trim = (ctx: any) => {
    if (!isLocal(ctx.model)) return;
    const active: string[] = pi.getActiveTools();
    // without Pi's search the deferred tools could not be reached at all
    if (!active.includes("tool_search")) return;
    const keep = active.filter((name) => !name.startsWith(PREFIX) || DIRECT.has(name) || loaded.has(name));
    if (keep.length !== active.length) pi.setActiveTools(keep);
  };

  pi.on("tool_result", (event: any) => {
    if (event.toolName !== "tool_search") return;
    for (const name of event.details?.loaded ?? []) loaded.add(name);
  });
  // T3 re-declares its whole catalog on each of these events
  pi.on("session_start", (_event: any, ctx: any) => trim(ctx));
  pi.on("model_select", (_event: any, ctx: any) => trim(ctx));
  pi.on("session_tree", (_event: any, ctx: any) => trim(ctx));
  pi.on("before_agent_start", (_event: any, ctx: any) => {
    trim(ctx);
  });
}
