// bun test opencode-wrap   (no OpenCode server needed: request/response translation only)
import { describe, expect, test } from "bun:test"
import { limit, parseOptions, parseToolCalls, renderHistory, toolInstruction } from "./server"

const msgs = [{ role: "user", content: "hi" }]

describe("parseOptions", () => {
  test("defaults", () => expect(parseOptions({ messages: msgs })).toEqual({ maxTokens: undefined, stop: undefined, responseFormat: undefined }))
  test("rejects empty messages", () => expect(() => parseOptions({ messages: [] })).toThrow("non-empty"))
  test("rejects n>1", () => expect(() => parseOptions({ messages: msgs, n: 2 })).toThrow("n=1"))
  test("range checks", () => expect(() => parseOptions({ messages: msgs, temperature: 3 })).toThrow("temperature"))
  test("max_completion_tokens wins", () =>
    expect(parseOptions({ messages: msgs, max_tokens: 5, max_completion_tokens: 7 }).maxTokens).toBe(7))
  test("stop string/array", () => {
    expect(parseOptions({ messages: msgs, stop: "END" }).stop).toEqual(["END"])
    expect(parseOptions({ messages: msgs, stop: ["a", "", "b", "c", "d", "e"] }).stop).toEqual(["a", "b", "c", "d"])
    expect(() => parseOptions({ messages: msgs, stop: [1] })).toThrow("stop")
  })
  test("response_format", () => {
    expect(parseOptions({ messages: msgs, response_format: { type: "json_object" } }).responseFormat).toBe("json_object")
    expect(parseOptions({ messages: msgs, response_format: { type: "text" } }).responseFormat).toBeUndefined()
    expect(() => parseOptions({ messages: msgs, response_format: { type: "xml" } })).toThrow("unsupported")
  })
  test("tool_choice", () => {
    expect(() => parseOptions({ messages: msgs, tool_choice: { function: { name: "f" } } })).not.toThrow()
    expect(() => parseOptions({ messages: msgs, tool_choice: "sometimes" })).toThrow("tool_choice")
  })
})

describe("tool calls", () => {
  const tools = [{ type: "function", function: { name: "get_weather", parameters: { type: "object" } } }]
  test("instruction lists tools and honors choice", () => {
    expect(toolInstruction(tools, undefined)).toContain('"name":"get_weather"')
    expect(toolInstruction(tools, "none")).toBe("")
    expect(toolInstruction(tools, "required")).toContain("MUST call at least one tool")
    expect(toolInstruction(tools, { function: { name: "get_weather" } })).toContain('MUST call the tool named "get_weather"')
  })
  test("parses known blocks, keeps the rest as content", () => {
    const text = 'Checking.\n```tool_call\n{"name":"get_weather","arguments":{"city":"Berlin"}}\n```\n```tool_call\n{"name":"rm_rf","arguments":{}}\n```'
    const { content, calls } = parseToolCalls(text, new Set(["get_weather"]))
    expect(calls).toEqual([{ name: "get_weather", args: { city: "Berlin" } }])
    expect(content).toStartWith("Checking.")
    expect(content).toContain("rm_rf") // unknown tool stays text
  })
  test("bad JSON is ignored", () => expect(parseToolCalls("```tool_call\n{oops}\n```", new Set()).calls).toEqual([]))
})

test("limit: stop and approximate max_tokens", () => {
  expect(limit("hello END world", { stop: ["END"] })).toEqual({ text: "hello ", truncatedBy: "stop" })
  expect(limit("abcdefghij", { maxTokens: 2 })).toEqual({ text: "abcdefgh", truncatedBy: "length" })
  expect(limit("short", {})).toEqual({ text: "short", truncatedBy: undefined })
})

test("renderHistory", () => {
  const out = renderHistory([
    { role: "system", content: "ignored" },
    { role: "user", content: [{ type: "text", text: "look" }, { type: "image_url", image_url: { url: "data:image/png;base64,AA" } }] },
    { role: "assistant", content: "", tool_calls: [{ id: "c1", function: { name: "f", arguments: "{}" } }] },
    { role: "tool", name: "f", tool_call_id: "c1", content: "42" },
  ])
  expect(out).not.toContain("ignored")
  expect(out).toContain("USER: look\n[attached 1 image/file part(s)")
  expect(out).toContain('ASSISTANT TOOL CALLS: [{"id":"c1","name":"f","arguments":"{}"}]')
  expect(out).toContain("TOOL RESULT (name=f id=c1):\n42")
})
