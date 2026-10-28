// OpenCode plugin: don't send T3 Code's MCP tools to local models (llama.cpp, Ollama).
// T3 Code adds its MCP server (~85 tools, about 29k tokens of schemas) to every OpenCode session. A local
// model's context can't hold that next to OpenCode's own prompt, so it forgets the task or stops calling
// tools. OpenCode has no tool search, so the local providers only keep T3's three core tools; cloud models
// keep all of them. setup.bat copies this to ~/.config/opencode/plugins/.
import { Plugin } from "@opencode/plugin"

const LOCAL_PROVIDERS = ["llama-cpp", "ollama"]
const CORE = ["orchestrator_capabilities", "delegate_task", "task_status"]

// T3's server is "t3-code-<id>", its tools "t3-code-<id>_<tool>"
const isT3 = (name: string) => name.startsWith("t3-code") || name.startsWith("t3_code")

export default Plugin.define({
  id: "local-tools",
  async setup(ctx) {
    for (const providerID of LOCAL_PROVIDERS) {
      await ctx.session.hook(
        "context",
        (event) => {
          for (const name of Object.keys(event.tools)) {
            if (isT3(name) && !CORE.some((tool) => name.endsWith("_" + tool))) delete event.tools[name]
          }
        },
        { providerID },
      )
    }
  },
})
