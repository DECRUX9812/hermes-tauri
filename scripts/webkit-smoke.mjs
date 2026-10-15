#!/usr/bin/env node

import { spawn } from 'node:child_process'
import { mkdir } from 'node:fs/promises'
import path from 'node:path'
import { fileURLToPath } from 'node:url'

const launchUrl = process.env.WEBKIT_SMOKE_URL
if (!launchUrl) {
  console.error('Set WEBKIT_SMOKE_URL to the private Hermes launch link.')
  process.exit(2)
}

const routes = [
  { name: 'home', hash: '#/' },
  { name: 'team', hash: '#/team' },
  { name: 'kanban', hash: '#/kanban' },
  { name: 'studio', hash: '#/studio' },
  { name: 'settings', hash: '#/settings' }
]
const outputDir = process.env.WEBKIT_SMOKE_OUTPUT ?? '/tmp/hermes-tauri-t8'
const playwrightModule = process.env.PLAYWRIGHT_MODULE ?? 'playwright'
const requestedEngines = (process.env.WEBKIT_SMOKE_ENGINES ?? 'webkit,chromium')
  .split(',')
  .map(name => name.trim())
const redact = value => String(value)
  .replace(/hermes-session=[A-Za-z0-9._~-]+/g, 'hermes-session=<redacted>')
  .replace(/ticket=[^&\s]+/g, 'ticket=<redacted>')

let playwright
try {
  playwright = await import(playwrightModule)
} catch (error) {
  console.error(`Could not load Playwright: ${redact(error.message)}`)
  process.exit(2)
}

await mkdir(outputDir, { recursive: true })
const summary = { screenshots: outputDir, engines: {} }

const engines = new Map([
  ['webkit', playwright.webkit],
  ['chromium', playwright.chromium]
])
const scriptDir = path.dirname(fileURLToPath(import.meta.url))

function runHostWebKitGtk() {
  return new Promise(resolve => {
    const child = spawn(
      'xvfb-run',
      ['-a', '-s', '-screen 0 1440x960x24', 'python3', path.join(scriptDir, 'webkitgtk-smoke.py')],
      { env: process.env, stdio: ['ignore', 'pipe', 'pipe'] }
    )
    let stdout = ''
    let stderr = ''
    child.stdout.on('data', chunk => { stdout += chunk })
    child.stderr.on('data', chunk => { stderr += chunk })
    child.on('error', error => resolve({ error: redact(error.message), stderr: redact(stderr) }))
    child.on('close', code => {
      if (code !== 0) {
        resolve({ error: `host WebKitGTK exited with ${code}`, stderr: redact(stderr) })
        return
      }
      try {
        resolve(JSON.parse(stdout))
      } catch (error) {
        resolve({ error: `host WebKitGTK returned invalid JSON: ${redact(error.message)}` })
      }
    })
  })
}

for (const name of requestedEngines) {
  const engine = engines.get(name)
  if (!engine) {
    console.error(`Unsupported browser engine: ${name}`)
    process.exit(2)
  }
  const result = { routes: {} }
  summary.engines[name] = result
  let browser

  try {
    browser = await engine.launch({ headless: true })
    result.version = browser.version()
  } catch (error) {
    result.launchError = redact(error.message)
    if (name === 'webkit' && process.env.WEBKIT_SMOKE_HOST_GTK !== '0') {
      const fallback = await runHostWebKitGtk()
      if (fallback.error) {
        result.hostFallbackError = fallback.error
        if (fallback.stderr) {
          result.hostFallbackStderr = fallback.stderr
        }
      } else {
        result.playwrightLaunchError = result.launchError
        delete result.launchError
        Object.assign(result, fallback)
      }
    }
    continue
  }

  try {
    const context = await browser.newContext({
      viewport: { width: 1440, height: 960 },
      deviceScaleFactor: 1
    })
    const page = await context.newPage()
    let currentRoute = 'boot'
    const routeResults = new Map()

    const errorsFor = route => {
      if (!routeResults.has(route)) {
        routeResults.set(route, { console: [], page: [] })
      }
      return routeResults.get(route)
    }

    page.on('console', message => {
      if (message.type() === 'error') {
        errorsFor(currentRoute).console.push(redact(message.text()))
      }
    })
    page.on('pageerror', error => {
      errorsFor(currentRoute).page.push(redact(error.stack ?? error.message))
    })

    try {
      await page.goto(launchUrl, { waitUntil: 'domcontentloaded', timeout: 30000 })
      await page.locator('body').waitFor({ state: 'attached', timeout: 15000 })
      await page.waitForFunction(
        () => !window.location.hash.startsWith('#hermes-session='),
        undefined,
        { timeout: 15000 }
      )

      const skipSetup = page.getByRole('button', {
        name: "I'll choose a provider later",
        exact: true
      })
      if (await skipSetup.count()) {
        await skipSetup.first().click()
        result.onboardingSkipped = true
        await page.waitForTimeout(1000)
      }

      for (const route of routes) {
        currentRoute = route.name
        errorsFor(currentRoute)
        await page.evaluate(hash => { window.location.hash = hash }, route.hash)
        await page.waitForTimeout(1200)
        const screenshot = path.join(outputDir, `${name}-${route.name}.png`)
        await page.screenshot({ path: screenshot, animations: 'disabled' })
        result.routes[route.name] = {
          screenshot: path.basename(screenshot),
          ...errorsFor(route.name)
        }
      }
    } catch (error) {
      result.runError = redact(error.stack ?? error.message)
    }

    result.boot = errorsFor('boot')
    await context.close()
  } catch (error) {
    result.runError = redact(error.stack ?? error.message)
  } finally {
    await browser.close()
  }
}

console.log(JSON.stringify(summary, null, 2))

if (Object.values(summary.engines).some(result => result.launchError || result.runError)) {
  process.exitCode = 1
}
