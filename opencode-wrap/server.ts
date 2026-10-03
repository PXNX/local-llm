#!/usr/bin/env bun
// OpenAI-compatible API in front of `opencode serve` (OpenCode 2), for the free OpenCode Zen models.
// Bun port of https://github.com/Fast-Editor/OpenCode-Wrap (Rust, OpenCode 1 API), adapted to the
// OpenCode 2 server API (/api/session/..., /api/event).
//   bun opencode-wrap/server.ts      ->  http://127.0.0.1:8000/v1
// No API key needed, reuses whatever auth the `opencode` CLI has (none for the free models).

import { mkdir, rm } from "node:fs/promises"

const env = (name: string, def: string) => process.env[name]?.trim() || def
function num(name: string, def: number, min: number, max: number) {
  const raw = process.env[name]?.trim()
  if (!raw) return def
  const n = Number(raw)
  if (!Number.isInteger(n) || n < min || n > max) throw new Error(`invalid ${name}=${raw} (expected ${min}..${max})`)
  return n
}

const WRAP_PORT = num("WRAP_PORT", 8000, 1, 65535)
const OPENCODE_PORT = num("OPENCODE_PORT", 4100, 1, 65535)
const OPENCODE_BASE = env("OPENCODE_BASE", `http://127.0.0.1:${OPENCODE_PORT}`).replace(/\/+$/, "")
// OpenCode 2 built by install-opencode2.bat; the `opencode` on PATH is usually 1.x (T3 Code's), which
// has a different server API
const OPENCODE2 = `${process.env.USERPROFILE || process.env.HOME}/.opencode2/bin/opencode${process.platform === "win32" ? ".exe" : ""}`
const OPENCODE_BIN = env("OPENCODE_BIN", (await Bun.file(OPENCODE2).exists()) ? OPENCODE2 : "opencode")
const DEFAULT_MODEL = env("WRAP_MODEL", "muse-spark-1.3-contributor-free")
const DEFAULT_PROVIDER = env("WRAP_PROVIDER", "opencode")
// cwd of the spawned serve: an empty folder, OpenCode's own tools (if a model uses them) run there
const WRAP_CWD = env("WRAP_CWD", `${process.env.TEMP || process.env.TMPDIR || "/tmp"}/opencode-wrap`).replaceAll("\\", "/")
const TIMEOUT_MS = num("WRAP_OCO_TIMEOUT_MS", 180_000, 100, 1_800_000)
const MAX_BODY = num("WRAP_MAX_BODY_BYTES", 8 * 1024 * 1024, 1024, 64 * 1024 * 1024)
const ZEN_MODELS_URL = env("WRAP_ZEN_MODELS_URL", "https://opencode.ai/zen/v1/models")
// OpenCode 2 servers always want basic auth (user "opencode"): OPENCODE_PASSWORD for an existing
// server, a spawned one gets a random password
const password = process.env.OPENCODE_PASSWORD || process.env.OPENCODE_SERVER_PASSWORD || crypto.randomUUID()
const AUTH = { Authorization: "Basic " + btoa(`opencode:${password}`) }

const log = (...a: unknown[]) => console.log("[wrap]", ...a)

// ---------- errors ----------

class WrapError extends Error {
  constructor(message: string, public status = 500, public type = "server_error", public code = "internal_error") {
    super(message)
  }
}
class Upstream extends WrapError {
  constructor(public upstreamStatus: number, public body: string) {
    super(`upstream ${upstreamStatus}: ${body.slice(0, 300)}`)
  }
}
const invalid = (message: string, code = "invalid_request_error", status = 400) =>
  new WrapError(message, status, "invalid_request_error", code)

const isRateLimited = (e: unknown) =>
  e instanceof Upstream && /rate|429|freeusagelimit|overloaded|capacity|too many requests/i.test(e.body)
const isTransient = (e: unknown) =>
  e instanceof Error && !(e instanceof WrapError) && /ECONNREFUSED|ECONNRESET|connect|fetch failed|timed? ?out|abort/i.test(e.message)
const isRetryable = (e: unknown) =>
  isTransient(e) || (e instanceof Upstream && !(e.upstreamStatus >= 400 && e.upstreamStatus < 500))

function httpStatus(e: unknown) {
  if (e instanceof WrapError && !(e instanceof Upstream)) return e.status
  if (isRateLimited(e)) return 429
  if (isTransient(e)) return 502
  if (e instanceof Upstream) return e.upstreamStatus >= 500 ? 502 : e.upstreamStatus
  return 500
}

function errorResponse(e: unknown) {
  const status = httpStatus(e)
  let type = "server_error", code = "internal_error", message = e instanceof Error ? e.message : String(e)
  if (e instanceof WrapError && !(e instanceof Upstream)) ({ type, code } = e)
  else if (status === 429) [type, code, message] = ["rate_limit_error", "rate_limit_exceeded", "Upstream model backend is rate-limited, retry with backoff."]
  else if (status === 502) [code, message] = ["bad_gateway", `Upstream model backend timed out / unavailable after retries (${message}).`]
  else if (e instanceof Upstream) [code, message] = ["upstream_error", `Upstream model backend failed after retries: ${e.body.slice(0, 200)}`]
  return Response.json({ error: { message, type, code } }, { status })
}

// ---------- opencode server ----------

async function oco(path: string, method = "GET", body?: unknown, timeoutMs = TIMEOUT_MS): Promise<any> {
  const res = await fetch(OPENCODE_BASE + path, {
    method,
    headers: { "Content-Type": "application/json", ...AUTH },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(timeoutMs),
  })
  const text = await res.text()
  if (!res.ok) throw new Upstream(res.status, text.slice(0, 2000))
  return text ? JSON.parse(text) : null
}

async function healthy() {
  try {
    return typeof (await oco("/api/info", "GET", undefined, 3000))?.version === "string"
  } catch {
    return false
  }
}

// the model catalog loads a few seconds after the server is up, requests before that fail
async function modelsReady() {
  try {
    const res = await oco("/api/model", "GET", undefined, 3000)
    return res?.data?.some((m: any) => m.providerID === DEFAULT_PROVIDER && m.id === resolveModelId(DEFAULT_MODEL))
  } catch {
    return false
  }
}

async function waitReady(ms: number) {
  const deadline = Date.now() + ms
  while (Date.now() < deadline) {
    if (child && child.exitCode !== null) throw new Error(`opencode serve exited with code ${child.exitCode}`)
    if ((await healthy()) && (await modelsReady())) return true
    await Bun.sleep(300)
  }
  return false
}

let child: Bun.Subprocess | undefined

async function ensureOpencode() {
  await mkdir(WRAP_CWD, { recursive: true })
  if (await healthy()) {
    if (!(await waitReady(30_000))) throw new Error(`model ${DEFAULT_PROVIDER}/${DEFAULT_MODEL} not available at ${OPENCODE_BASE}`)
    return log(`using existing opencode serve at ${OPENCODE_BASE}`)
  }
  const version = Bun.spawnSync([OPENCODE_BIN, "--version"]).stdout.toString().trim()
  if (!/^(opencode )?v?([2-9]|\d{2,})\./.test(version))
    throw new Error(`${OPENCODE_BIN} is "${version || "missing"}", opencode-wrap needs OpenCode 2: run opencode-wrap/install-opencode2.bat or set OPENCODE_BIN`)
  log(`spawning \`${OPENCODE_BIN} serve\` on :${OPENCODE_PORT} (cwd=${WRAP_CWD}) ...`)
  try {
    child = Bun.spawn([OPENCODE_BIN, "serve", "--port", String(OPENCODE_PORT), "--hostname", "127.0.0.1"], {
      cwd: WRAP_CWD,
      env: { ...process.env, OPENCODE_PASSWORD: password },
      stdin: "ignore",
      stdout: "inherit",
      stderr: "inherit",
    })
  } catch (e) {
    throw new Error(`failed to spawn \`${OPENCODE_BIN} serve\`: ${e}. Is OpenCode 2 installed? (setup.bat)`)
  }
  if (!(await waitReady(60_000))) throw new Error("opencode serve did not become ready in 60s")
  log("opencode serve ready")
}

// ---------- Zen model list (cached 1 h) ----------

let zen = { at: 0, ids: new Set<string>() }
async function zenIds() {
  if (Date.now() - zen.at < 3_600_000 && zen.ids.size) return zen.ids
  try {
    const res = await fetch(ZEN_MODELS_URL, { signal: AbortSignal.timeout(15_000) })
    if (!res.ok) throw new Error(`status ${res.status}`)
    const ids = new Set<string>(((await res.json()) as any).data?.map((m: any) => m.id).filter(Boolean))
    if (ids.size) zen = { at: Date.now(), ids }
  } catch (e) {
    log(`zen models fetch failed (${e}), skipping validation`)
  }
  return zen.ids
}

const resolveModelId = (raw: string) => {
  const id = raw.trim()
  return /^muse-spark-.+-contributor$/.test(id) ? `${id}-free` : id
}

async function listModels() {
  const created = Math.floor(Date.now() / 1000)
  const def = `${DEFAULT_PROVIDER}/${resolveModelId(DEFAULT_MODEL)}`
  const data = [{ id: def, object: "model", created, owned_by: "opencode-wrap" }]
  for (const id of [...(await zenIds())].sort()) {
    const full = `opencode/${id}`
    if (full !== def) data.push({ id: full, object: "model", created, owned_by: "opencode" })
  }
  return { object: "list", data }
}

// ---------- OpenAI request -> prompt ----------

type Msg = { role?: string; content?: unknown; name?: string; tool_call_id?: string; tool_calls?: any[] }
type Opts = { maxTokens?: number; stop?: string[]; responseFormat?: string }

function partText(p: any): string {
  if (typeof p === "string") return p
  if (!p || typeof p !== "object") return ""
  if (typeof p.text === "string" && (!p.type || p.type === "text" || p.type === "input_text")) return p.text
  if (typeof p.input_text === "string") return p.input_text
  if (p.type === "text" && typeof p.content === "string") return p.content
  return ""
}
const msgText = (m: Msg) =>
  typeof m.content === "string" ? m.content : Array.isArray(m.content) ? m.content.map(partText).join("") : ""

function partUrl(p: any): string | undefined {
  const u = (v: any) => (typeof v === "string" ? v : undefined)
  switch (p?.type) {
    case "image_url": return u(p.image_url?.url) ?? u(p.image_url)
    case "image": return u(p.url) ?? u(p.image_url)
    case "file": return u(p.url) ?? u(p.file?.url)
    case "input_image": return u(p.image_url) ?? u(p.url)
  }
}

function guessMime(url: string) {
  const data = /^data:([^;,]+)?(;base64)?,/.exec(url)
  if (data?.[1]) return data[1].toLowerCase()
  const clean = url.split(/[?#]/)[0].toLowerCase()
  const ext: Record<string, string> = { png: "image/png", jpg: "image/jpeg", jpeg: "image/jpeg", webp: "image/webp", gif: "image/gif", pdf: "application/pdf", mp4: "video/mp4" }
  return ext[clean.split(".").pop() ?? ""] ?? "image/png"
}

const fileParts = (messages: Msg[]) =>
  messages.flatMap((m) =>
    Array.isArray(m.content)
      ? m.content.map(partUrl).filter((u): u is string => !!u).map((url) => ({ url, mime: guessMime(url) }))
      : [],
  )

function renderHistory(messages: Msg[]) {
  const out: string[] = []
  for (const m of messages) {
    const role = m.role ?? ""
    if (role === "system" || role === "developer") continue
    const imgs = Array.isArray(m.content) ? m.content.filter((p: any) => partUrl(p)).length : 0
    const suffix = imgs ? `\n[attached ${imgs} image/file part(s) — see message attachments]` : ""
    if (role === "tool") {
      out.push(`TOOL RESULT (name=${m.name ?? "?"} id=${m.tool_call_id ?? "?"}):\n${msgText(m)}${suffix}`)
    } else if (role === "assistant" && m.tool_calls) {
      const text = msgText(m)
      if (text || suffix) out.push(`ASSISTANT: ${text ? text + suffix : suffix.trim()}`)
      const calls = m.tool_calls.map((t) => ({ id: t.id, name: t.function?.name, arguments: t.function?.arguments }))
      out.push(`ASSISTANT TOOL CALLS: ${JSON.stringify(calls)}`)
    } else {
      out.push(`${role.toUpperCase() || "USER"}: ${msgText(m)}${suffix}`)
    }
  }
  return out.join("\n\n")
}

function parseOptions(body: any): Opts {
  if (!Array.isArray(body.messages) || !body.messages.length) throw invalid("messages must be a non-empty array")
  if (body.n != null && body.n !== 1) throw invalid("only n=1 is supported")
  const range = (v: unknown, name: string, min: number, max: number) => {
    if (v != null && (typeof v !== "number" || !Number.isFinite(v) || v < min || v > max))
      throw invalid(`${name} must be a number ${min}..${max}`)
  }
  range(body.temperature, "temperature", 0, 2)
  range(body.top_p, "top_p", 0, 1)
  range(body.presence_penalty, "presence_penalty", -2, 2)
  range(body.frequency_penalty, "frequency_penalty", -2, 2)

  const max = body.max_completion_tokens ?? body.max_tokens
  if (max != null && (!Number.isInteger(max) || max <= 0)) throw invalid("max_tokens must be a positive integer")

  let stop: string[] | undefined
  if (typeof body.stop === "string") stop = body.stop ? [body.stop] : undefined
  else if (Array.isArray(body.stop)) {
    if (body.stop.some((s: unknown) => typeof s !== "string")) throw invalid("stop must be a string or array of strings")
    stop = body.stop.filter(Boolean).slice(0, 4)
    if (!stop!.length) stop = undefined
  } else if (body.stop != null) throw invalid("stop must be a string or array of strings")

  let responseFormat: string | undefined
  if (body.response_format != null) {
    const t = body.response_format.type
    if (typeof t !== "string") throw invalid("response_format.type must be a string")
    if (t === "json_object" || t === "json_schema") responseFormat = t
    else if (t !== "text") throw invalid(`unsupported response_format.type "${t}"`)
  }

  const tc = body.tool_choice
  if (tc != null && !["none", "auto", "required"].includes(tc) && typeof tc?.function?.name !== "string")
    throw invalid('tool_choice must be "none", "auto", "required", or {function:{name}}')

  return { maxTokens: max ?? undefined, stop, responseFormat }
}

function toolInstruction(tools: any[], choice: any) {
  if (!tools.length || choice === "none") return ""
  const defs = tools.map((t) => {
    if (t.type !== "function") return t
    const f = t.function ?? t
    return { name: f.name, description: f.description ?? "", parameters: f.parameters ?? {} }
  })
  const rule =
    typeof choice?.function?.name === "string"
      ? `You MUST call the tool named "${choice.function.name}" — reply with its tool call block. Do not answer in plain text.`
      : choice === "required"
        ? "You MUST call at least one tool — reply with tool call block(s). Do not answer in plain text."
        : "If the request needs a tool, reply with one or more tool call blocks. If no tool is needed, answer normally with no blocks."
  return [
    "You have access to these tools (OpenAI function format):",
    JSON.stringify(defs),
    `RULES: ${rule}`,
    "Each call is exactly one fenced block, nothing else inside the block:",
    "```tool_call",
    '{"name":"<tool name>","arguments":{...}}',
    "```",
    "Use only the listed tool names. arguments must be a JSON object matching the tool's parameters schema. Put any explanation OUTSIDE the blocks.",
  ].join("\n")
}

function parseToolCalls(text: string, known: Set<string>) {
  const calls: { name: string; args: object }[] = []
  let rest = text
  for (const m of text.matchAll(/```tool_call\s*([\s\S]*?)```/g)) {
    try {
      const obj = JSON.parse(m[1].trim())
      if (typeof obj?.name !== "string" || (known.size && !known.has(obj.name))) continue
      const args = obj.arguments && typeof obj.arguments === "object" && !Array.isArray(obj.arguments) ? obj.arguments : {}
      calls.push({ name: obj.name, args })
      rest = rest.replace(m[0], "")
    } catch {}
  }
  return { content: rest.trim(), calls }
}

function limit(text: string, opts: Opts) {
  let out = text
  let truncatedBy: "stop" | "length" | undefined
  const cuts = (opts.stop ?? []).map((s) => out.indexOf(s)).filter((i) => i >= 0)
  if (cuts.length) [out, truncatedBy] = [out.slice(0, Math.min(...cuts)), "stop"]
  // no per-request token limit in the backend: approximate with 4 chars per token
  if (opts.maxTokens && out.length > opts.maxTokens * 4) [out, truncatedBy] = [out.slice(0, opts.maxTokens * 4), "length"]
  return { text: out, truncatedBy }
}

type Turn = { modelName: string; model: { providerID: string; id: string }; prompt: string; files: { url: string; mime: string }[]; known: Set<string>; opts: Opts }

async function prepareTurn(body: any): Promise<Turn> {
  const messages: Msg[] = body.messages
  const opts = parseOptions(body)
  const raw = typeof body.model === "string" && body.model.trim() ? body.model.trim() : `${DEFAULT_PROVIDER}/${DEFAULT_MODEL}`
  const slash = raw.indexOf("/")
  const explicit = slash > 0
  const reqProvider = explicit ? raw.slice(0, slash).trim() : DEFAULT_PROVIDER
  const reqId = resolveModelId(explicit ? raw.slice(slash + 1) : raw)

  const ids = await zenIds()
  let model: Turn["model"]
  if (ids.has(reqId) || (explicit && reqProvider === "opencode" && !ids.size)) model = { providerID: "opencode", id: reqId }
  else if (explicit && reqProvider === "opencode") {
    const needle = reqId.replace("-free", "")
    const sug = [...ids].filter((id) => id.includes(needle)).slice(0, 3)
    throw invalid(`unknown Zen model "${reqId}"${sug.length ? `, did you mean: ${sug.join(", ")}?` : ""}`, "model_not_found")
  } else {
    // virtual names (e.g. "gpt-4o" from a client that insists on one) go to the default model
    log(`virtual model "${raw}" -> backend ${DEFAULT_PROVIDER}/${DEFAULT_MODEL}`)
    model = { providerID: DEFAULT_PROVIDER, id: resolveModelId(DEFAULT_MODEL) }
  }

  const tools: any[] = Array.isArray(body.tools) ? body.tools : []
  const known = new Set<string>(tools.filter((t) => t.type === "function" && t.function?.name).map((t) => t.function.name))
  const system = messages.filter((m) => m.role === "system" || m.role === "developer").map(msgText).join("\n\n")
  const last = messages[messages.length - 1]
  const turn = last.role === "tool" || last.role === "assistant" ? msgText(last) : `USER: ${msgText(last)}`
  const format = { json_object: "You MUST reply with a single valid JSON object and nothing else (no markdown fences, no prose).", json_schema: "You MUST reply with valid JSON matching the requested schema and nothing else (no markdown fences, no prose)." }[opts.responseFormat ?? ""] ?? ""
  const schema = opts.responseFormat === "json_schema" && body.response_format.json_schema?.schema ? `Schema: ${JSON.stringify(body.response_format.json_schema.schema)}` : ""
  const systemText = [system, toolInstruction(tools, body.tool_choice), format, schema].filter(Boolean).join("\n\n")
  const promptText = [renderHistory(messages.slice(0, -1)), turn].filter(Boolean).join("\n\n")

  return {
    modelName: typeof body.model === "string" ? body.model : `${model.providerID}/${model.id}`,
    model,
    // the generate route has no separate system field, so instructions lead the prompt
    prompt: systemText ? `SYSTEM:\n${systemText}\n\n${promptText}` : promptText,
    files: fileParts(messages),
    known,
    opts,
  }
}

// ---------- backend calls ----------
// The free Zen tier only answers requests that come from inside OpenCode (a session with its tools),
// so the stateless /api/experimental/generate route is rejected. Two session-based paths instead:
//   text, no stream:      session + /generate - one model call, no agent loop, nothing is executed
//   stream or images:     session + /prompt, answer read live from the shared /api/event stream
// Images become temp files (file:// attachments are passed to the model as images, data: URIs are not).

// One SSE connection to /api/event for all requests, events are routed by session ID.
type SessionEvent = { type: string; data: any }
const listeners = new Map<string, (e: SessionEvent) => void>()
let connected: Promise<void> | undefined

// resolves once the stream is connected, then keeps it open (reconnects when it drops)
function ensureEvents() {
  return (connected ??= new Promise<void>((ready) => {
    ;(async () => {
      for (;;) {
        try {
          const res = await fetch(`${OPENCODE_BASE}/api/event`, { headers: AUTH })
          if (!res.ok || !res.body) throw new Error(`status ${res.status}`)
          ready()
          let buf = ""
          for await (const chunk of res.body.pipeThrough(new TextDecoderStream())) {
            buf += chunk
            let end: number
            while ((end = buf.indexOf("\n\n")) >= 0) {
              const block = buf.slice(0, end)
              buf = buf.slice(end + 2)
              for (const line of block.split("\n")) {
                if (!line.startsWith("data:")) continue
                try {
                  const e = JSON.parse(line.slice(5))
                  const sid = e?.data?.sessionID
                  if (sid) listeners.get(sid)?.(e)
                } catch {}
              }
            }
          }
        } catch (e) {
          log(`event stream dropped (${e}), reconnecting ...`)
        }
        await Bun.sleep(500)
      }
    })()
  }))
}

async function newSession(model: Turn["model"]): Promise<string> {
  const session = await oco("/api/session", "POST", { model, title: "opencode-wrap" })
  const sid: string | undefined = session?.data?.id
  if (!sid) throw new Upstream(502, "missing session id")
  return sid
}
const dropSession = (sid: string) => void oco(`/api/session/${sid}`, "DELETE", undefined, 15_000).catch(() => {})

// OpenCode picks the mime type from the file extension, so it comes from the bytes (clients often
// label every image image/jpeg)
function sniffExt(b: Uint8Array, mime: string) {
  const hex = Buffer.from(b.subarray(0, 12)).toString("hex")
  if (hex.startsWith("89504e47")) return "png"
  if (hex.startsWith("ffd8ff")) return "jpg"
  if (hex.startsWith("47494638")) return "gif"
  if (hex.startsWith("52494646") && hex.slice(16, 24) === "57454250") return "webp"
  if (hex.startsWith("25504446")) return "pdf"
  return mime.split("/")[1]?.replace(/[^a-z0-9]/g, "") || "bin"
}

async function writeFiles(files: Turn["files"]) {
  const dir = `${WRAP_CWD}/opencode-wrap-${crypto.randomUUID()}`
  const attachments = await Promise.all(
    files.map(async (f, i) => {
      const data = /^data:[^,]*;base64,(.*)$/s.exec(f.url)
      let bytes: Uint8Array
      if (data) bytes = Buffer.from(data[1], "base64")
      else if (/^https?:/.test(f.url)) {
        const res = await fetch(f.url, { signal: AbortSignal.timeout(30_000) })
        if (!res.ok) throw invalid(`could not download ${f.url}: status ${res.status}`)
        bytes = new Uint8Array(await res.arrayBuffer())
      } else throw invalid("image/file parts must be data: or http(s): URLs")
      const name = `attachment-${i + 1}.${sniffExt(bytes, f.mime)}`
      await Bun.write(`${dir}/${name}`, bytes)
      return { uri: Bun.pathToFileURL(`${dir}/${name}`).href, name }
    }),
  )
  return { dir, attachments }
}

async function generate(turn: Turn): Promise<string> {
  const sid = await newSession(turn.model)
  try {
    const res = await oco(`/api/session/${sid}/generate`, "POST", { prompt: turn.prompt })
    return res?.data?.text ?? ""
  } finally {
    dropSession(sid)
  }
}

// Runs the prompt and calls onDelta for every text delta, resolves with the final answer text.
async function promptTurn(turn: Turn, onDelta?: (text: string) => void): Promise<string> {
  await ensureEvents()
  const sid = await newSession(turn.model)
  const files = turn.files.length ? await writeFiles(turn.files) : undefined
  const texts = new Map<string, string>() // assistantMessageID:ordinal -> text, the last one is the answer
  let last = ""
  try {
    return await new Promise<string>((resolve, reject) => {
      const timer = setTimeout(() => reject(Object.assign(new Error("timed out"), { name: "TimeoutError" })), TIMEOUT_MS)
      listeners.set(sid, (e) => {
        const d = e.data
        const key = `${d.assistantMessageID}:${d.ordinal}`
        if (e.type === "session.text.delta") {
          texts.set(key, (texts.get(key) ?? "") + d.delta)
          last = key
          onDelta?.(d.delta)
        } else if (e.type === "session.text.ended") {
          texts.set(key, d.text)
          last = key
        } else if (e.type === "session.execution.succeeded") {
          clearTimeout(timer)
          resolve(texts.get(last) ?? "")
        } else if (e.type === "session.execution.failed" || e.type === "session.execution.interrupted") {
          clearTimeout(timer)
          const err = d.error
          reject(new Upstream(err?.status ?? 502, err?.response?.body ?? err?.message ?? e.type))
        }
      })
      const text = turn.files.length ? `${turn.prompt}\n\nAnswer from the attached file(s) directly, do not use tools.` : turn.prompt
      oco(`/api/session/${sid}/prompt`, "POST", { text, files: files?.attachments }).catch((e) => {
        clearTimeout(timer)
        reject(e)
      })
    })
  } finally {
    listeners.delete(sid)
    dropSession(sid)
    if (files) rm(files.dir, { recursive: true, force: true }).catch(() => {})
  }
}

async function withRetry<T>(fn: () => Promise<T>, attempts = 3): Promise<T> {
  for (let attempt = 1; ; attempt++) {
    try {
      return await fn()
    } catch (e) {
      log(`attempt ${attempt}/${attempts} failed: ${e instanceof Error ? e.message : e}`)
      if (!isRetryable(e) || attempt >= attempts) throw e
      await Bun.sleep(isRateLimited(e) ? 4000 : 1500 * attempt)
    }
  }
}

const nonEmpty = (text: string) => {
  if (!text.trim()) throw new Upstream(502, "empty completion")
  return text
}
const answer = (turn: Turn) =>
  withRetry(async () => nonEmpty(turn.files.length ? await promptTurn(turn) : await generate(turn)))

// ---------- OpenAI responses ----------

const rid = (prefix: string) => `${prefix}_${Date.now().toString(16)}${Math.floor(Math.random() * 2 ** 32).toString(16)}`

function completion(turn: Turn, raw: string) {
  const { content, calls } = parseToolCalls(raw, turn.known)
  const limited = limit(content || (calls.length ? "" : raw), turn.opts)
  const toolCalls = calls.map((c) => ({ id: rid("call"), type: "function", function: { name: c.name, arguments: JSON.stringify(c.args) } }))
  const finish = calls.length ? "tool_calls" : limited.truncatedBy === "length" ? "length" : "stop"
  // the backend reports no token counts on these routes: estimate with 4 chars per token
  const promptTokens = Math.ceil(turn.prompt.length / 4)
  const completionTokens = Math.ceil(raw.length / 4)
  return {
    id: rid("chatcmpl"),
    created: Math.floor(Date.now() / 1000),
    model: turn.modelName,
    content: limited.text,
    toolCalls,
    finish,
    usage: { prompt_tokens: promptTokens, completion_tokens: completionTokens, total_tokens: promptTokens + completionTokens },
  }
}

async function chat(body: any) {
  const turn = await prepareTurn(body)
  const c = completion(turn, await answer(turn))
  const message = c.toolCalls.length
    ? { role: "assistant", content: c.content || null, tool_calls: c.toolCalls }
    : { role: "assistant", content: c.content }
  return Response.json({
    id: c.id,
    object: "chat.completion",
    created: c.created,
    model: c.model,
    choices: [{ index: 0, message, finish_reason: c.finish }],
    usage: c.usage,
  })
}

// Text deltas are forwarded as they arrive. With tools in the request the text is held back until
// the end, so ```tool_call blocks can be turned into tool_calls instead of leaking as content.
async function chatStream(body: any) {
  const turn = await prepareTurn(body) // validation errors become a normal 4xx JSON response
  const enc = new TextEncoder()
  const id = rid("chatcmpl")
  const base = { id, object: "chat.completion.chunk", created: Math.floor(Date.now() / 1000), model: turn.modelName }
  const live = !turn.known.size && !turn.opts.stop && !turn.opts.maxTokens
  return new Response(
    new ReadableStream({
      async start(ctl) {
        let open = true
        const send = (s: string) => {
          if (!open) return
          try {
            ctl.enqueue(enc.encode(s))
          } catch {
            open = false // client went away
          }
        }
        const chunk = (delta: object, finish: string | null = null) =>
          send(`data: ${JSON.stringify({ ...base, choices: [{ index: 0, delta, finish_reason: finish }] })}\n\n`)
        const keepAlive = setInterval(() => send(": keep-alive\n\n"), 10_000)
        try {
          chunk({ role: "assistant", content: "" })
          let sent = false
          const raw = await withRetry(async () => {
            if (sent) throw new WrapError("stream interrupted after output started", 502, "server_error", "bad_gateway")
            return nonEmpty(await promptTurn(turn, live ? (delta) => (sent = true, chunk({ content: delta })) : undefined))
          })
          const c = completion(turn, raw)
          if (!live) for (let i = 0; i < c.content.length; i += 256) chunk({ content: c.content.slice(i, i + 256) })
          if (c.toolCalls.length) chunk({ tool_calls: c.toolCalls.map((t, index) => ({ index, ...t })) })
          chunk({}, c.finish)
          send(`data: ${JSON.stringify({ ...base, choices: [], usage: c.usage })}\n\n`)
        } catch (e) {
          send(`data: ${JSON.stringify({ error: { message: e instanceof Error ? e.message : String(e) } })}\n\n`)
        } finally {
          clearInterval(keepAlive)
          send("data: [DONE]\n\n")
          if (open) ctl.close()
        }
      },
    }),
    { headers: { "Content-Type": "text/event-stream", "Cache-Control": "no-cache", Connection: "keep-alive" } },
  )
}

// ---------- HTTP server ----------

const CORS = { "Access-Control-Allow-Origin": "*", "Access-Control-Allow-Methods": "*", "Access-Control-Allow-Headers": "*" }
let inFlight = 0
let shuttingDown = false

async function handle(req: Request): Promise<Response> {
  const { pathname } = new URL(req.url)
  if (req.method === "OPTIONS") return new Response(null, { status: 204 })
  if (req.method === "GET" && (pathname === "/health" || pathname === "/v1/health"))
    return Response.json({ status: "ok", upstream: OPENCODE_BASE, spawned: !!child })
  if (req.method === "GET" && pathname === "/v1/models") return Response.json(await listModels())
  if (req.method !== "POST" || pathname !== "/v1/chat/completions")
    return Response.json({ error: { message: `no route ${req.method} ${pathname}`, type: "invalid_request_error", code: "not_found" } }, { status: 404 })
  if (shuttingDown) return errorResponse(new WrapError("server shutting down", 503, "server_error", "shutting_down"))
  if (Number(req.headers.get("content-length") ?? 0) > MAX_BODY) return errorResponse(invalid("request body too large", "body_too_large", 413))

  let body: any
  try {
    body = await req.json()
  } catch {
    return errorResponse(invalid("request body must be JSON"))
  }
  log(`chat: model=${body.model} msgs=${body.messages?.length ?? 0} tools=${body.tools?.length ?? 0} stream=${!!body.stream}`)
  inFlight++
  try {
    return body.stream === true ? await chatStream(body) : await chat(body)
  } catch (e) {
    return errorResponse(e)
  } finally {
    inFlight--
  }
}

if (import.meta.main) {
  await ensureOpencode()
  const server = Bun.serve({
    hostname: "127.0.0.1",
    port: WRAP_PORT,
    maxRequestBodySize: MAX_BODY,
    idleTimeout: 255, // seconds (Bun's maximum); free models can take minutes
    async fetch(req) {
      const res = await handle(req)
      for (const [k, v] of Object.entries(CORS)) res.headers.set(k, v)
      return res
    },
  })
  log(`OpenAI-compatible API at http://127.0.0.1:${server.port}/v1`)

  const shutdown = async () => {
    shuttingDown = true
    if (inFlight) log(`draining ${inFlight} in-flight request(s)...`)
    while (inFlight) await Bun.sleep(100)
    server.stop()
    child?.kill()
    process.exit(0)
  }
  process.on("SIGINT", shutdown)
  process.on("SIGTERM", shutdown)
}

export { completion, limit, parseOptions, parseToolCalls, renderHistory, toolInstruction }
