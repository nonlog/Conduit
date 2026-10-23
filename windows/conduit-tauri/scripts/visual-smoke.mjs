import { spawn } from "node:child_process"
import { existsSync, mkdirSync } from "node:fs"
import { resolve } from "node:path"
import assert from "node:assert/strict"
import { chromium } from "playwright-core"

const url = "http://127.0.0.1:1420/"
const project = resolve(import.meta.dirname, "..")
const shots = resolve(project, "artifacts", "browser-smoke")
const browsers = [
  process.env.CONDUIT_TEST_BROWSER,
  "C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe",
  "C:\\Program Files\\Microsoft\\Edge\\Application\\msedge.exe",
  "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
  "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
].filter(Boolean)
const executable = browsers.find((candidate) => existsSync(candidate))
if (!executable) throw new Error("No Windows Edge/Chrome executable available for preview smoke.")

mkdirSync(shots, { recursive: true })
const ready = async () => {
  try { return (await fetch(url, { signal: AbortSignal.timeout(750) })).ok }
  catch { return false }
}
let server
if (!(await ready())) {
  server = spawn(process.execPath, ["run", "dev", "--host", "127.0.0.1"], {
    cwd: project, stdio: "ignore", windowsHide: true,
  })
  server.on("error", console.error)
  server.unref()
  for (let i = 0; i < 90; i++) {
    if (await ready()) break
    await new Promise((done) => setTimeout(done, 400))
  }
  if (!(await ready())) throw new Error("Vite preview did not become ready")
}

const errors = []
let browser
try {
  browser = await chromium.launch({
    executablePath: executable,
    headless: true,
    args: ["--no-first-run", "--disable-gpu"],
  })
  const page = await browser.newPage({ viewport: { width: 1360, height: 900 }, deviceScaleFactor: 1 })
  page.on("pageerror", (error) => errors.push(error.message))
  await page.goto(url, { waitUntil: "networkidle", timeout: 30000 })
  const workspace = page.getByRole("navigation", { name: "Workspace" })
  assert.equal(await page.getByRole("heading", { name: "Overview" }).count(), 1)
  assert.equal(await page.getByText("Preview phone").count() > 0, true)
  assert.equal(await page.getByText("design-specification.pdf").count(), 0, "Completed transfers must not occupy the overview")
  assert.equal(await page.locator(".active-strip").count(), 0)
  await page.screenshot({ path: resolve(shots, "overview-light.png") })

  await workspace.getByRole("button", { name: "Notifications", exact: true }).click()
  await page.getByRole("heading", { name: "Notifications" }).waitFor()
  assert.equal(await page.getByText("Project update").count(), 1)
  await page.screenshot({ path: resolve(shots, "notifications-light.png") })
  await page.getByRole("textbox", { name: "Search current section" }).fill("calendar")
  assert.equal(await page.getByText("Project update").count(), 0)
  assert.equal(await page.getByText("Team sync").count(), 1)

  await workspace.getByRole("button", { name: "Shared links", exact: true }).click()
  await page.getByRole("heading", { name: "Shared links" }).waitFor()
  assert.equal(await page.getByText("Conduit on GitHub").count(), 1)
  await page.screenshot({ path: resolve(shots, "links-light.png") })

  await workspace.getByRole("button", { name: "File transfers", exact: true }).click()
  await page.getByRole("heading", { name: "File transfers" }).waitFor()
  assert.equal(await page.getByText("design-specification.pdf").count(), 1)
  assert.equal(await page.locator(".active-strip").count(), 0)
  await page.screenshot({ path: resolve(shots, "transfers-light.png") })

  await workspace.getByRole("button", { name: "Settings", exact: true }).click()
  await page.getByRole("heading", { name: "Settings" }).waitFor()
  const first = await page.getByText("Start at sign-in").boundingBox()
  const second = await page.getByText("Explorer integration").boundingBox()
  assert.ok(first && second && second.y >= first.y + first.height, "Settings rows overlap")
  await page.screenshot({ path: resolve(shots, "settings-light.png") })

  // Regression: the native UI formerly retained Settings' scroll position when
  // switching to Transfers, hiding the destination's title and newest rows.
  const scrollContainer = page.locator("#content")
  await scrollContainer.evaluate((element) => { element.scrollTop = element.scrollHeight })
  assert.ok(await scrollContainer.evaluate((element) => element.scrollTop) > 100, "Settings must be scrollable")
  await workspace.getByRole("button", { name: "File transfers", exact: true }).click()
  await page.getByRole("heading", { name: "File transfers" }).waitFor()
  await page.waitForFunction(() => document.querySelector("#content")?.scrollTop === 0)
  await workspace.getByRole("button", { name: "Settings", exact: true }).click()
  await page.getByRole("heading", { name: "Settings" }).waitFor()

  await page.getByRole("button", { name: "Toggle light or dark theme" }).click()
  assert.equal(await page.locator("html.dark").count(), 1, "Dark theme did not activate")
  await page.screenshot({ path: resolve(shots, "settings-dark.png") })

  await workspace.getByRole("button", { name: "About", exact: true }).click()
  await page.getByRole("heading", { name: "About" }).waitFor()
  await page.screenshot({ path: resolve(shots, "about-dark.png") })

  await workspace.getByRole("button", { name: "Overview", exact: true }).click()
  await page.getByRole("heading", { name: "Overview" }).waitFor()
  await page.setViewportSize({ width: 860, height: 660 })
  const metrics = page.locator(".overview-metric")
  assert.equal(await metrics.count(), 3)
  const scrollWidth = await page.evaluate(() => document.documentElement.scrollWidth)
  assert.ok(scrollWidth <= 862, "Preview overflows a narrow desktop window")
  await page.screenshot({ path: resolve(shots, "overview-dark-narrow.png") })
  assert.deepEqual(errors, [], "Browser JavaScript errors")
  console.log("Nova browser smoke passed. Saved 8 previews to " + shots)
} finally {
  if (browser) await browser.close()
  if (server) server.kill("SIGTERM")
}
