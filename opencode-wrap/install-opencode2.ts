#!/usr/bin/env bun
// Builds OpenCode 2 from source into ~/.opencode2/bin/opencode(.exe) - a single native binary
// (bun build --compile). OpenCode 2 is tagged upstream (v2.x) but not published to npm/releases yet,
// the npm package opencode-ai is still 1.x. Safe to re-run: skips the build when that version is
// installed already.
//   bun opencode-wrap/install-opencode2.ts            latest v2.x tag
//   bun opencode-wrap/install-opencode2.ts v2.0.22    a specific tag
import { $ } from "bun"
import { copyFile, mkdir, rm } from "node:fs/promises"
import { homedir, tmpdir } from "node:os"
import { join } from "node:path"

const REPO = "https://github.com/anomalyco/opencode"
const exe = process.platform === "win32" ? ".exe" : ""
const binDir = join(homedir(), ".opencode2", "bin")
const target = join(binDir, `opencode${exe}`)

async function latestV2() {
  const refs = await $`git ls-remote --tags --refs ${REPO} ${"v2.*"}`.text()
  const tags = refs.split("\n").map((l) => l.split("refs/tags/")[1]).filter((t) => /^v2\.\d+\.\d+$/.test(t ?? ""))
  const key = (t: string) => t.slice(1).split(".").map(Number)
  tags.sort((a, b) => {
    const [x, y] = [key(a), key(b)]
    return x[0] - y[0] || x[1] - y[1] || x[2] - y[2]
  })
  if (!tags.length) throw new Error("no v2.x tags found upstream")
  return tags[tags.length - 1]
}

const tag = process.argv[2] ?? (await latestV2())
const version = tag.replace(/^v/, "")
if (await Bun.file(target).exists()) {
  const have = (await $`${target} --version`.nothrow().text()).trim()
  if (have.endsWith(version)) {
    console.log(`OpenCode ${version} is installed already: ${target}`)
    process.exit(0)
  }
  console.log(`Installed: ${have || "unknown"}, building ${version} ...`)
}

const src = join(tmpdir(), `opencode-${tag}`)
await rm(src, { recursive: true, force: true })
console.log(`=== clone ${REPO} ${tag}`)
await $`git clone --depth 1 --branch ${tag} ${REPO} ${src}`
console.log("=== bun install (a few minutes the first time)")
await $`bun install`.cwd(src)
console.log("=== build (bun build --compile, this platform only)")
await $`bun run script/build.ts --single --skip-install --skip-web-ui`
  .cwd(join(src, "packages", "cli"))
  .env({ ...process.env, OPENCODE_VERSION: version, OPENCODE_CHANNEL: "latest" })

const built = (await Array.fromAsync(new Bun.Glob(`dist/*/bin/opencode${exe}`).scan({ cwd: join(src, "packages", "cli") })))[0]
if (!built) throw new Error("build finished but no binary found in packages/cli/dist")
await mkdir(binDir, { recursive: true })
await copyFile(join(src, "packages", "cli", built), target)
await rm(src, { recursive: true, force: true }).catch(() => {})
console.log(`OpenCode ${(await $`${target} --version`.text()).trim()} -> ${target}`)
