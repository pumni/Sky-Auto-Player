import { spawn, spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { createServer } from "node:net";
import { createRequire } from "node:module";
import {
  existsSync,
  mkdirSync,
  mkdtempSync,
  readFileSync as readBinary,
  readFileSync as readText,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { setTimeout as delay } from "node:timers/promises";

const repoRoot = resolve(dirname(fileURLToPath(import.meta.url)), "../../..");
const require = createRequire(join(repoRoot, "desktop", "package.json"));
const { chromium } = require("playwright");
const registryPath = String.raw`HKCU:\Software\Microsoft\Windows\CurrentVersion\Themes\Personalize`;
const options = Object.fromEntries(
  process.argv.slice(2).map((arg) => {
    const [key, ...parts] = arg.replace(/^--/, "").split("=");
    return [key, parts.join("=") || true];
  }),
);

if (process.platform !== "win32")
  throw new Error("This qualification requires Windows.");
if (options["allow-os-app-theme-toggle"] !== true)
  throw new Error(
    "Pass --allow-os-app-theme-toggle to authorize a temporary Windows AppsUseLightTheme change.",
  );

const exe = resolve(
  String(
    options.exe ?? join(repoRoot, "rust/target/dist/sky_desktop_shell.exe"),
  ),
);
const outputDir = resolve(
  String(options.output ?? join(repoRoot, "docs/evidence/issue-565-package-c")),
);
if (!existsSync(exe)) throw new Error(`Packaged executable not found: ${exe}`);
mkdirSync(outputDir, { recursive: true });

function run(command, args, cwd = repoRoot) {
  const result = spawnSync(command, args, {
    cwd,
    encoding: "utf8",
    windowsHide: true,
  });
  if (result.status !== 0)
    throw new Error(
      `${command} ${args.join(" ")} failed (${result.status}): ${result.stderr.trim()}`,
    );
  return result.stdout.trim();
}

function runPowerShell(script) {
  return run("powershell.exe", [
    "-NoProfile",
    "-NonInteractive",
    "-ExecutionPolicy",
    "Bypass",
    "-Command",
    script,
  ]);
}

const broadcast = String.raw`
Add-Type -TypeDefinition @'
using System;
using System.Runtime.InteropServices;
public static class Issue565ThemeBroadcast {
  [DllImport("user32.dll", CharSet=CharSet.Unicode, SetLastError=true)]
  public static extern IntPtr SendMessageTimeout(IntPtr hWnd, UInt32 message, UIntPtr wParam,
    String lParam, UInt32 flags, UInt32 timeout, out UIntPtr result);
}
'@
$result = [UIntPtr]::Zero
[void][Issue565ThemeBroadcast]::SendMessageTimeout([IntPtr](-1), 0x001A, [UIntPtr]::Zero,
  'ImmersiveColorSet', 0x0002, 5000, [ref]$result)
`;

function readWindowsAppTheme() {
  const script = String.raw`
$path = '${registryPath}'
$item = Get-ItemProperty -Path $path -ErrorAction SilentlyContinue
$app = if ($null -eq $item) { $null } else { $item.PSObject.Properties['AppsUseLightTheme'] }
$system = if ($null -eq $item) { $null } else { $item.PSObject.Properties['SystemUsesLightTheme'] }
[pscustomobject]@{
  apps_value_exists = ($null -ne $app)
  apps_use_light_theme = if ($null -eq $app) { $null } else { [int]$app.Value }
  system_value_exists = ($null -ne $system)
  system_use_light_theme = if ($null -eq $system) { $null } else { [int]$system.Value }
} | ConvertTo-Json -Compress
`;
  return JSON.parse(runPowerShell(script));
}

function setWindowsAppTheme(mode) {
  const value = mode === "light" ? 1 : 0;
  const script = String.raw`
$path = '${registryPath}'
New-Item -Path $path -Force | Out-Null
New-ItemProperty -Path $path -Name AppsUseLightTheme -PropertyType DWord -Value ${value} -Force | Out-Null
${broadcast}
`;
  runPowerShell(script);
}

function restoreWindowsAppTheme(original) {
  const script = String.raw`
$path = '${registryPath}'
New-Item -Path $path -Force | Out-Null
if (${original.apps_value_exists ? "$true" : "$false"}) {
  New-ItemProperty -Path $path -Name AppsUseLightTheme -PropertyType DWord -Value ${original.apps_use_light_theme ?? 0} -Force | Out-Null
} else { Remove-ItemProperty -Path $path -Name AppsUseLightTheme -ErrorAction SilentlyContinue }
if (${original.system_value_exists ? "$true" : "$false"}) {
  New-ItemProperty -Path $path -Name SystemUsesLightTheme -PropertyType DWord -Value ${original.system_use_light_theme ?? 0} -Force | Out-Null
} else { Remove-ItemProperty -Path $path -Name SystemUsesLightTheme -ErrorAction SilentlyContinue }
${broadcast}
`;
  runPowerShell(script);
}

async function availablePort() {
  const server = createServer();
  await new Promise((resolveListen, reject) => {
    server.once("error", reject);
    server.listen(0, "127.0.0.1", resolveListen);
  });
  const address = server.address();
  if (!address || typeof address === "string")
    throw new Error("No loopback port available.");
  await new Promise((resolveClose, reject) =>
    server.close((error) => (error ? reject(error) : resolveClose())),
  );
  return address.port;
}

async function startApp(appDataRoot) {
  const sandbox = join(appDataRoot, "webview-profile");
  const port = await availablePort();
  const child = spawn(exe, [], {
    cwd: repoRoot,
    windowsHide: true,
    stdio: "ignore",
    env: {
      ...process.env,
      SKY_INSTALL_ROOT: repoRoot,
      SKY_APP_DATA_ROOT: appDataRoot,
      WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${port}`,
      WEBVIEW2_USER_DATA_FOLDER: sandbox,
    },
  });
  let browser;
  for (let attempt = 0; attempt < 120; attempt += 1) {
    try {
      browser = await chromium.connectOverCDP(`http://127.0.0.1:${port}`);
      break;
    } catch (error) {
      if (child.exitCode !== null)
        throw new Error(
          `Shell exited before WebView2 connected: ${child.exitCode}`,
        );
      if (attempt === 119) throw error;
      await delay(250);
    }
  }
  const page = browser.contexts()[0]?.pages()[0];
  if (!page) throw new Error("The packaged WebView2 has no page.");
  await page.locator(".workbench").waitFor({ timeout: 30_000 });
  const isTauri = await page.evaluate(() =>
    Boolean(window.__TAURI_INTERNALS__),
  );
  if (!isTauri)
    throw new Error("The connected page is not the packaged Tauri WebView.");
  return { child, browser, page };
}

async function stopApp(session) {
  if (!session) return;
  try {
    await session.page
      .locator('button[aria-label="Close window"]')
      .evaluate((element) => element.click());
  } catch {
    // The process-tree fallback below is limited to this harness-launched PID.
  }
  const exited = await new Promise((resolveExit) => {
    if (session.child.exitCode !== null) return resolveExit(true);
    const timer = setTimeout(() => resolveExit(false), 8_000);
    session.child.once("exit", () => {
      clearTimeout(timer);
      resolveExit(true);
    });
  });
  if (!exited && session.child.exitCode === null) {
    run("taskkill.exe", ["/PID", String(session.child.pid), "/T", "/F"]);
  }
  await session.browser.close().catch(() => undefined);
}

function readConfig(appDataRoot) {
  return JSON.parse(readText(join(appDataRoot, "config.json"), "utf8"));
}

async function waitForConfig(appDataRoot, expected) {
  for (let attempt = 0; attempt < 100; attempt += 1) {
    try {
      const config = readConfig(appDataRoot);
      if (
        config.appearance_mode === expected.appearance_mode &&
        config.palette === expected.palette
      )
        return config;
    } catch {
      // Native config may not have been flushed yet.
    }
    await delay(100);
  }
  throw new Error(`Native config did not persist ${JSON.stringify(expected)}.`);
}

async function currentAppearance(page) {
  return page.evaluate(() => ({
    mode: document.documentElement.dataset.colorMode,
    palette: document.documentElement.dataset.palette,
    colorScheme: getComputedStyle(document.documentElement).colorScheme,
    devicePixelRatio,
    visualViewportScale: window.visualViewport?.scale ?? null,
    rootZoom: getComputedStyle(document.documentElement).zoom,
    viewport: { width: innerWidth, height: innerHeight },
    htmlBackground: getComputedStyle(document.documentElement).backgroundColor,
    bodyBackground: getComputedStyle(document.body).backgroundColor,
    tauriBridge: Boolean(window.__TAURI_INTERNALS__),
    geometry: Object.fromEntries(
      [
        ".app-titlebar",
        ".track-browser-workbench-pane",
        ".track-row",
        ".player-bar",
      ].map((selector) => {
        const rect = document.querySelector(selector)?.getBoundingClientRect();
        return [
          selector,
          rect ? { width: rect.width, height: rect.height } : null,
        ];
      }),
    ),
    documentSize: {
      width: document.documentElement.scrollWidth,
      height: document.documentElement.scrollHeight,
    },
    localStorageThemeKeys: Object.keys(localStorage).filter((key) =>
      /appearance|color.?mode|theme/i.test(key),
    ),
  }));
}

async function renderedTextContrast(page) {
  const samples = await page.evaluate(() => {
    const parse = (value) => {
      const channels = value.match(/[\d.]+/g)?.map(Number) ?? [];
      return {
        r: channels[0] ?? 0,
        g: channels[1] ?? 0,
        b: channels[2] ?? 0,
        a: channels.length > 3 ? channels[3] : value === "transparent" ? 0 : 1,
      };
    };
    const composite = (front, back) => {
      const alpha = front.a + back.a * (1 - front.a);
      if (!alpha) return { r: 255, g: 255, b: 255, a: 1 };
      return {
        r: (front.r * front.a + back.r * back.a * (1 - front.a)) / alpha,
        g: (front.g * front.a + back.g * back.a * (1 - front.a)) / alpha,
        b: (front.b * front.a + back.b * back.a * (1 - front.a)) / alpha,
        a: alpha,
      };
    };
    const luminance = (color) => {
      const linear = [color.r, color.g, color.b].map((channel) => {
        const value = channel / 255;
        return value <= 0.04045
          ? value / 12.92
          : ((value + 0.055) / 1.055) ** 2.4;
      });
      return linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722;
    };
    const ratio = (first, second) => {
      const a = luminance(first);
      const b = luminance(second);
      return Number(
        ((Math.max(a, b) + 0.05) / (Math.min(a, b) + 0.05)).toFixed(2),
      );
    };
    const samples = [];
    const walker = document.createTreeWalker(
      document.body,
      NodeFilter.SHOW_TEXT,
    );
    while (walker.nextNode()) {
      const node = walker.currentNode;
      const text = node.textContent?.replace(/\s+/g, " ").trim();
      const element = node.parentElement;
      if (
        !text ||
        !element ||
        element.closest('[aria-hidden="true"], .visually-hidden')
      )
        continue;
      const style = getComputedStyle(element);
      const bounds = element.getBoundingClientRect();
      if (
        style.display === "none" ||
        style.visibility === "hidden" ||
        bounds.width <= 0 ||
        bounds.height <= 0 ||
        bounds.right <= 0 ||
        bounds.bottom <= 0 ||
        bounds.left >= innerWidth ||
        bounds.top >= innerHeight
      )
        continue;
      const ancestors = [];
      for (let current = element; current; current = current.parentElement)
        ancestors.push(current);
      let background = { r: 255, g: 255, b: 255, a: 1 };
      let opacity = 1;
      for (const ancestor of ancestors.reverse()) {
        const ancestorStyle = getComputedStyle(ancestor);
        opacity *= Number.parseFloat(ancestorStyle.opacity || "1");
        const color = parse(ancestorStyle.backgroundColor);
        color.a *= opacity;
        background = composite(color, background);
      }
      const foreground = parse(style.color);
      foreground.a *= opacity;
      const rendered = composite(foreground, background);
      const fontSize = Number.parseFloat(style.fontSize);
      const fontWeight = Number.parseInt(style.fontWeight, 10) || 400;
      const minimum =
        fontSize >= 24 || (fontSize >= 18.67 && fontWeight >= 700) ? 3 : 4.5;
      const disabled = element.closest(':disabled, [aria-disabled="true"]');
      samples.push({
        text: text.slice(0, 72),
        element: `${element.tagName.toLowerCase()}.${String(
          element.className || "",
        )
          .trim()
          .replace(/\s+/g, ".")}`,
        ratio: ratio(rendered, background),
        minimum,
        exemption: disabled ? "inactive-control" : null,
      });
    }
    return samples;
  });
  const applicable = samples.filter((sample) => !sample.exemption);
  const failures = applicable.filter((sample) => sample.ratio < sample.minimum);
  if (failures.length)
    throw new Error(
      `Rendered text contrast failures: ${JSON.stringify(failures)}`,
    );
  return {
    visibleTextSamples: samples.length,
    inactiveControlExemptions: samples.length - applicable.length,
    minimumRatio: Math.min(...applicable.map((sample) => sample.ratio)),
    failures,
    lowestSamples: [...applicable]
      .sort((a, b) => a.ratio - b.ratio)
      .slice(0, 5),
  };
}

function contrastRatio(foreground, background) {
  const luminance = (value) => {
    const channels =
      value
        .match(/[\d.]+/g)
        ?.map(Number)
        .slice(0, 3) ?? [];
    return channels.reduce((sum, channel, index) => {
      const normalized = channel / 255;
      const linear =
        normalized <= 0.04045
          ? normalized / 12.92
          : ((normalized + 0.055) / 1.055) ** 2.4;
      return sum + linear * [0.2126, 0.7152, 0.0722][index];
    }, 0);
  };
  const first = luminance(foreground);
  const second = luminance(background);
  return Number(
    (
      (Math.max(first, second) + 0.05) /
      (Math.min(first, second) + 0.05)
    ).toFixed(2),
  );
}

async function focusGlobalSearch(page) {
  const alreadyFocused = await page.evaluate(() => {
    const input = document.querySelector('input[type="search"]');
    const wrapper = document.querySelector(".global-search");
    return Boolean(
      input &&
      wrapper &&
      document.activeElement === input &&
      input.matches(":focus-visible") &&
      wrapper.matches(":focus-within") &&
      wrapper.matches(":has(input:focus-visible)"),
    );
  });
  if (!alreadyFocused) {
    await page.waitForFunction(
      () => !document.querySelector('[role="dialog"][aria-label="Settings"]'),
      undefined,
      { timeout: 5_000 },
    );
    await page.waitForFunction(
      () =>
        document.activeElement?.getAttribute("aria-label") === "Open settings",
      undefined,
      { timeout: 5_000 },
    );
    await page.keyboard.press("Control+f");
  }
  let stableFocusSamples = 0;
  for (let attempt = 0; attempt < 40 && stableFocusSamples < 4; attempt += 1) {
    const focused = await page.evaluate(() => {
      const input = document.querySelector('input[type="search"]');
      const wrapper = document.querySelector(".global-search");
      return Boolean(
        input &&
        wrapper &&
        document.activeElement === input &&
        input.matches(":focus-visible") &&
        wrapper.matches(":focus-within") &&
        wrapper.matches(":has(input:focus-visible)"),
      );
    });
    stableFocusSamples = focused ? stableFocusSamples + 1 : 0;
    if (stableFocusSamples < 4) await delay(50);
  }
  if (stableFocusSamples < 4)
    throw new Error("Keyboard focus did not settle on Global Search.");
  const focus = await page.locator(".global-search").evaluate((element) => {
    const style = getComputedStyle(element);
    const input = element.querySelector('input[type="search"]');
    const ancestors = [];
    for (let current = element; current; current = current.parentElement)
      ancestors.push(current);
    let background = "rgb(255, 255, 255)";
    for (const ancestor of ancestors) {
      const color = getComputedStyle(ancestor).backgroundColor;
      if (color !== "transparent" && color !== "rgba(0, 0, 0, 0)") {
        background = color;
        break;
      }
    }
    return {
      activeElementIsSearch: document.activeElement === input,
      inputFocusVisible: input?.matches(":focus-visible") ?? false,
      wrapperFocusWithin: element.matches(":focus-within"),
      wrapperHasFocusVisible: element.matches(":has(input:focus-visible)"),
      outlineColor: style.outlineColor,
      outlineStyle: style.outlineStyle,
      outlineWidth: style.outlineWidth,
      devicePixelRatio,
      background,
      forcedColors: matchMedia("(forced-colors: active)").matches,
      reducedMotion: matchMedia("(prefers-reduced-motion: reduce)").matches,
    };
  });
  focus.ratio = contrastRatio(focus.outlineColor, focus.background);
  focus.physicalOutlineWidth =
    Number.parseFloat(focus.outlineWidth) * focus.devicePixelRatio;
  const mode = await page.locator("html").getAttribute("data-color-mode");
  const expectedOutline =
    mode === "light" ? "rgb(11, 88, 165)" : "rgb(150, 136, 198)";
  if (
    !focus.activeElementIsSearch ||
    !focus.inputFocusVisible ||
    !focus.wrapperFocusWithin ||
    !focus.wrapperHasFocusVisible ||
    focus.outlineColor !== expectedOutline ||
    focus.outlineStyle !== "solid" ||
    Math.abs(focus.physicalOutlineWidth - 2) > 0.1 ||
    focus.ratio < 3
  )
    throw new Error(
      `Packaged WebView2 Global Search focus ring failed: ${JSON.stringify(focus)}`,
    );
  return focus;
}

async function expectMode(page, expected, label) {
  await page.waitForFunction(
    (mode) => document.documentElement.dataset.colorMode === mode,
    expected,
    { timeout: 10_000 },
  );
  const actual = await currentAppearance(page);
  if (actual.mode !== expected)
    throw new Error(`${label}: expected ${expected}, got ${actual.mode}`);
  return actual;
}

async function openAppearance(page) {
  const dialog = page.getByRole("dialog", { name: "Settings" });
  if ((await dialog.count()) === 0)
    await page.getByRole("button", { name: "Open settings" }).click();
  await dialog.waitFor();
  await dialog.getByRole("button", { name: "Appearance", exact: true }).click();
  const theme = dialog.getByLabel("Theme");
  const palette = dialog.getByLabel("Dark palette");
  await theme.waitFor();
  return { dialog, theme, palette };
}

async function selectMode(page, mode) {
  const { theme } = await openAppearance(page);
  await theme.selectOption(mode);
  const expected =
    mode === "system"
      ? (await readWindowsAppTheme()).apps_use_light_theme
        ? "light"
        : "dark"
      : mode;
  return expectMode(page, expected, `Settings ${mode}`);
}

function captureHash(path) {
  return createHash("sha256").update(readBinary(path)).digest("hex");
}

const sourceSha = run("git", ["rev-parse", "HEAD"]);
const sourceBranch = run("git", ["branch", "--show-current"]);
const sourceStatus = run("git", ["status", "--porcelain=v1"]);
const nativeBuild = JSON.parse(run(exe, ["--selftest-build-info"]));
const originalWindowsTheme = readWindowsAppTheme();
const environment = JSON.parse(
  runPowerShell(
    "Add-Type -AssemblyName System.Windows.Forms; [pscustomobject]@{ os=(Get-CimInstance Win32_OperatingSystem | Select-Object -First 1 Caption,Version,BuildNumber); webview=(Get-ItemProperty 'HKLM:\\SOFTWARE\\WOW6432Node\\Microsoft\\EdgeUpdate\\Clients\\*' -ErrorAction SilentlyContinue | Where-Object { $_.name -match 'WebView2' } | Select-Object -First 1 name,pv); display=[pscustomobject]@{ primaryBounds=[System.Windows.Forms.Screen]::PrimaryScreen.Bounds.ToString(); primaryBitsPerPixel=[System.Windows.Forms.Screen]::PrimaryScreen.BitsPerPixel; videoMode=(Get-CimInstance Win32_VideoController | Select-Object -First 1 CurrentHorizontalResolution,CurrentVerticalResolution,CurrentRefreshRate) } } | ConvertTo-Json -Compress -Depth 4",
  ),
);
const tauriConfig = JSON.parse(
  readText(join(repoRoot, "desktop/src-tauri/tauri.conf.json"), "utf8"),
);
const mainWindowConfig = tauriConfig.app.windows.find(
  (window) => window.label === "main",
);
const sandbox = mkdtempSync(join(tmpdir(), "sky-issue-565-package-c-"));
const results = {
  captured_at: new Date().toISOString(),
  source: {
    sha: sourceSha,
    branch: sourceBranch,
    worktree_porcelain: sourceStatus || "clean",
  },
  executable: {
    path: exe,
    sha256: captureHash(exe),
    native_build: nativeBuild,
  },
  environment: {
    os: environment.os,
    webview2: environment.webview,
    display: environment.display,
    native_window_background: mainWindowConfig.backgroundColor,
    node: process.version,
    architecture: process.arch,
    test_harness:
      "Packaged Windows Tauri release executable attached to its actual WebView2 via CDP.",
    theme_toggle:
      "HKCU AppsUseLightTheme DWord + WM_SETTINGCHANGE ImmersiveColorSet broadcast; the harness restores both app/system values from the pre-test snapshot.",
    original_windows_app_theme: originalWindowsTheme,
    no_gameplay_input: true,
  },
  checks: [],
  screenshots: [],
  restoration: null,
  failure: null,
};
let session;

try {
  setWindowsAppTheme("light");
  const freshRoot = join(sandbox, "fresh-profile");
  mkdirSync(freshRoot);
  session = await startApp(freshRoot);
  const bridgeInfo = await currentAppearance(session.page);
  if (!bridgeInfo.tauriBridge)
    throw new Error("Fresh profile did not run in Tauri WebView2.");
  const freshStart = await expectMode(
    session.page,
    "light",
    "fresh profile follows Windows Apps Light",
  );
  const freshSettings = await openAppearance(session.page);
  if ((await freshSettings.theme.inputValue()) !== "system")
    throw new Error("Fresh profile Settings did not default to System.");
  if ((await freshSettings.palette.inputValue()) !== "graphite")
    throw new Error("Fresh profile palette was not Graphite.");
  await waitForConfig(freshRoot, {
    appearance_mode: "system",
    palette: "graphite",
  });
  if (freshStart.localStorageThemeKeys.length !== 0)
    throw new Error(
      `Appearance was also persisted in localStorage: ${freshStart.localStorageThemeKeys}`,
    );
  results.checks.push({
    name: "fresh profile follows Windows Apps Light, native System/Graphite config",
    pass: true,
    appearance: freshStart,
  });

  setWindowsAppTheme("dark");
  const liveDark = await expectMode(
    session.page,
    "dark",
    "System follows Windows Apps Dark",
  );
  setWindowsAppTheme("light");
  const liveLight = await expectMode(
    session.page,
    "light",
    "System follows Windows Apps Light",
  );
  results.checks.push({
    name: "live Windows Apps Light ↔ Dark events update System mode",
    pass: true,
    transitions: [liveDark, liveLight],
  });

  await selectMode(session.page, "dark");
  setWindowsAppTheme("dark");
  await delay(750);
  const manualDarkAfterDark = await currentAppearance(session.page);
  setWindowsAppTheme("light");
  await delay(750);
  const manualDarkAfterLight = await currentAppearance(session.page);
  if (
    manualDarkAfterDark.mode !== "dark" ||
    manualDarkAfterLight.mode !== "dark"
  )
    throw new Error(
      "Manual Dark did not remain independent of Windows app theme.",
    );
  await waitForConfig(freshRoot, {
    appearance_mode: "dark",
    palette: "graphite",
  });
  results.checks.push({
    name: "manual Dark ignores Windows app theme changes and persists natively",
    pass: true,
  });

  await selectMode(session.page, "light");
  setWindowsAppTheme("dark");
  await delay(750);
  const manualLightAfterDark = await currentAppearance(session.page);
  if (manualLightAfterDark.mode !== "light")
    throw new Error("Manual Light followed Windows Dark unexpectedly.");
  await waitForConfig(freshRoot, {
    appearance_mode: "light",
    palette: "graphite",
  });
  await selectMode(session.page, "system");
  const systemDark = await expectMode(
    session.page,
    "dark",
    "System resumes Windows Apps Dark",
  );
  setWindowsAppTheme("light");
  const systemLight = await expectMode(
    session.page,
    "light",
    "System resumes Windows Apps Light",
  );
  const lightSettings = await openAppearance(session.page);
  const lightSettingsContrast = await renderedTextContrast(session.page);
  await session.page.screenshot({
    path: join(outputDir, "fresh-system-light-webview2.png"),
    fullPage: true,
  });
  results.screenshots.push({
    file: "fresh-system-light-webview2.png",
    view: await currentAppearance(session.page),
    theme: await lightSettings.theme.inputValue(),
    contrast: lightSettingsContrast,
  });
  await waitForConfig(freshRoot, {
    appearance_mode: "system",
    palette: "graphite",
  });
  if (
    lightSettings.theme.inputValue &&
    (await lightSettings.theme.inputValue()) !== "system"
  )
    throw new Error(
      "System mode was not selected before fresh-profile restart.",
    );
  results.checks.push({
    name: "manual Light ignores Windows Dark; returning to System resumes live following",
    pass: true,
    transitions: [manualLightAfterDark, systemDark, systemLight],
  });

  await lightSettings.dialog
    .getByRole("button", { name: "Close settings" })
    .click();
  const lightFocus = await focusGlobalSearch(session.page);
  await session.page.screenshot({
    path: join(outputDir, "fresh-light-search-focus-webview2.png"),
    fullPage: true,
  });
  results.screenshots.push({
    file: "fresh-light-search-focus-webview2.png",
    view: await currentAppearance(session.page),
    focus: lightFocus,
  });
  setWindowsAppTheme("dark");
  await expectMode(
    session.page,
    "dark",
    "System updates to Dark while Search remains focused",
  );
  const darkFocus = await focusGlobalSearch(session.page);
  const darkContrast = await renderedTextContrast(session.page);
  await session.page.screenshot({
    path: join(outputDir, "fresh-system-dark-focus-webview2.png"),
    fullPage: true,
  });
  results.screenshots.push({
    file: "fresh-system-dark-focus-webview2.png",
    view: await currentAppearance(session.page),
    focus: darkFocus,
    contrast: darkContrast,
  });
  results.checks.push({
    name: "Global Search keyboard focus remains visible in packaged WebView2 Light and Dark",
    pass: true,
    light: lightFocus,
    dark: darkFocus,
  });
  setWindowsAppTheme("light");
  await expectMode(
    session.page,
    "light",
    "System returns to Light before restart",
  );

  await stopApp(session);
  session = null;
  session = await startApp(freshRoot);
  const freshRestart = await expectMode(
    session.page,
    "light",
    "fresh System preference survives restart",
  );
  await waitForConfig(freshRoot, {
    appearance_mode: "system",
    palette: "graphite",
  });
  results.checks.push({
    name: "fresh System preference and Graphite palette survive restart",
    pass: true,
    appearance: freshRestart,
  });
  await stopApp(session);
  session = null;

  const legacyRoot = join(sandbox, "legacy-profile");
  mkdirSync(legacyRoot);
  writeFileSync(
    join(legacyRoot, "config.json"),
    JSON.stringify({ schema_version: 9, palette: "slate" }),
  );
  session = await startApp(legacyRoot);
  const legacyStart = await expectMode(
    session.page,
    "dark",
    "legacy config keeps Dark on Windows Apps Light",
  );
  if (legacyStart.palette !== "slate")
    throw new Error(`Legacy palette changed: ${legacyStart.palette}`);
  const legacyConfig = await waitForConfig(legacyRoot, {
    appearance_mode: "dark",
    palette: "slate",
  });
  const legacySettings = await openAppearance(session.page);
  if (
    (await legacySettings.theme.inputValue()) !== "dark" ||
    (await legacySettings.palette.inputValue()) !== "slate"
  )
    throw new Error("Legacy Settings did not retain Dark and Slate.");
  await session.page.screenshot({
    path: join(outputDir, "legacy-dark-windows-light-webview2.png"),
    fullPage: true,
  });
  results.screenshots.push({
    file: "legacy-dark-windows-light-webview2.png",
    view: legacyStart,
    migrated_config: legacyConfig,
  });
  results.checks.push({
    name: "legacy schema 9 config migrates to Dark and retains Slate while Windows Apps is Light",
    pass: true,
    appearance: legacyStart,
    migrated_config: legacyConfig,
  });
  await stopApp(session);
  session = null;
  session = await startApp(legacyRoot);
  const legacyRestart = await expectMode(
    session.page,
    "dark",
    "legacy Dark and Slate survive restart",
  );
  if (legacyRestart.palette !== "slate")
    throw new Error("Legacy Slate palette did not survive restart.");
  await waitForConfig(legacyRoot, {
    appearance_mode: "dark",
    palette: "slate",
  });
  results.checks.push({
    name: "legacy Dark and Slate survive packaged restart",
    pass: true,
    appearance: legacyRestart,
  });
} catch (error) {
  results.failure = String(error);
} finally {
  let cleanupError;
  try {
    await stopApp(session);
  } catch (error) {
    cleanupError = error;
  }
  try {
    restoreWindowsAppTheme(originalWindowsTheme);
    const restoredWindowsTheme = readWindowsAppTheme();
    results.restoration = {
      restored:
        JSON.stringify(restoredWindowsTheme) ===
        JSON.stringify(originalWindowsTheme),
      after: restoredWindowsTheme,
    };
  } catch (error) {
    results.restoration = { restored: false, error: String(error) };
    cleanupError ??= error;
  }
  const reportPath = join(outputDir, "windows-webview2-qualification.json");
  writeFileSync(reportPath, `${JSON.stringify(results, null, 2)}\n`);
  console.log(
    JSON.stringify(
      {
        report: reportPath,
        checks: results.checks,
        screenshots: results.screenshots,
        restoration: results.restoration,
      },
      null,
      2,
    ),
  );
  if (!results.restoration.restored)
    cleanupError ??= new Error(
      "Windows theme registry values were not restored.",
    );
  if (cleanupError) results.failure ??= String(cleanupError);
}
if (results.failure) throw new Error(results.failure);
