# Starfish engine — command support matrix

`agent-browser --engine starfish` drives a headless [Starfish](https://github.com/) WebView
over its experimental CDP server. Starfish implements only a subset of the Chrome
DevTools Protocol, so some agent-browser commands work fully, some run but have no
real effect, and a few are unsupported.

This page maps each command group to its status. It was produced by probing the
`out/headless/bin/Starfish` build (Mock graphics backend) live, cross-checked
against the Starfish CDP behavior notes in `starfish-cdp-skill/ANALYSIS.md`.

## Setup

```bash
# Point the engine at a built Starfish binary, then use it like any other engine.
export STARFISH_BIN=/path/to/starfish/out/headless/bin/Starfish
agent-browser --engine starfish open https://example.com
agent-browser --engine starfish snapshot -i
agent-browser --engine starfish close

# AGENT_BROWSER_ENGINE=starfish makes it the default for the session.
```

Binary lookup order: `STARFISH_BIN` → `--executable-path` → `Starfish`/`starfish`
on `PATH` → `out/headless/bin/Starfish` (and other common build outputs) relative
to the working directory.

## Legend

| Mark | Meaning |
| --- | --- |
| ✅ | Supported — verified working against Starfish |
| 🟡 | Partial — the command runs (acks) but the effect is limited, blank, or unverified on Starfish |
| ❌ | Unsupported — errors or is a no-op |

## Engine-wide caveats

These shape every row below, so read them first:

- **Mock graphics backend.** `screenshot`, `screenshot --annotate`, `pdf`, the
  streaming viewport, and `diff screenshot` produce structurally valid output but
  the pixels are blank/transparent. Element labels and dimensions are real; the
  rendered image is not.
- **Single shared, persistent WebView.** Page state (URL, DOM, cookies, storage,
  history) persists across reconnects. Spawned tabs/windows are **not** isolated JS
  contexts, and console capture is **initial-tab only**.
- **Keep-alive is automatic.** The headless shell exits after `onload` if a page
  has no pending timer; the engine re-injects a no-op timer after every navigation,
  so multi-step sessions stay alive.
- **`Emulation.*` is largely ack-only.** Viewport/device/geolocation/color-scheme
  overrides are accepted but have no guaranteed effect on the engine.
- **Opaque origins have no storage.** `data:` / `about:` pages cannot read or write
  `localStorage`/`sessionStorage`; use an `http(s)` origin.

## Lifecycle & connection

| Command | Status | Notes |
| --- | --- | --- |
| `open` / `goto` / `navigate` | ✅ | `Page.navigate`; full HTTP + `data:` navigation, history tracked |
| `close` / `close --all` | ✅ | Kills the managed Starfish process; no orphan left |
| `connect <port>` / `--cdp` | ✅ | Can also attach to an already-running Starfish CDP endpoint |
| `--session` / `session list` | ✅ | agent-browser-level; engine-agnostic |
| `batch` | ✅ | engine-agnostic |
| `install` / `upgrade` / `doctor` | ❌ | Chrome-specific (downloads/repairs Chrome). Supply the Starfish binary yourself via `STARFISH_BIN` |

## Perceive

| Command | Status | Notes |
| --- | --- | --- |
| `snapshot` (`-i`, `-c`, `-d`, `-s`, `--urls`) | ✅ | `Accessibility.getFullAXTree`, synthesized from the DOM; refs work |
| `get text` / `html` / `value` / `title` / `url` / `attr` / `count` | ✅ | via `Runtime.evaluate` / DOM |
| `get box` | ✅ | `DOM.getBoxModel` content quad |
| `get cdp-url` | ✅ | |
| `get styles` | 🟡 | Returns an empty computed-style map on Starfish |
| `is visible` / `enabled` / `checked` | ✅ | |
| `eval` | ✅ | `Runtime.evaluate`, returnByValue + awaitPromise |
| `console` | ✅ | `Runtime.consoleAPICalled`; **initial tab only** |
| `errors` | ✅ | uncaught exceptions |
| `screenshot` / `screenshot --annotate` | 🟡 | Valid PNG + correct labels/dimensions, but **blank pixels** |
| `pdf` | 🟡 | Valid `%PDF-` document, blank content |

## Act

| Command | Status | Notes |
| --- | --- | --- |
| `click` / `dblclick` / `hover` / `focus` | ✅ | `Input.dispatchMouseEvent` at box-model coords |
| `type` / `fill` | ✅ | `DOM.focus` + `Input.insertText` |
| `press` / `keydown` / `keyup` / `keyboard` | ✅ | `Input.dispatchKeyEvent` |
| `mouse move` / `down` / `up` / `wheel` | ✅ | raw `Input.dispatchMouseEvent` |
| `select` / `check` / `uncheck` | ✅ | |
| `scroll` / `scrollintoview` | ✅ | |
| `highlight` | ✅ | Runs (no visible pixels on headless) |
| `find role/text/label/placeholder/alt/title/testid/first/last/nth` | ✅ | resolved via snapshot/DOM |
| `upload` | 🟡 | `DOM.setFileInputFiles` is reached; best-effort, requires a real file input |
| `drag` | 🟡 | `Input.dispatchDragEvent` is **ack-only** — no real drag-and-drop effect |

## Navigate & wait

| Command | Status | Notes |
| --- | --- | --- |
| `back` / `forward` / `reload` | ✅ | `Page.getNavigationHistory` + `navigateToHistoryEntry` |
| `pushstate` | ✅ | history.pushState / framework router |
| `wait` (selector / ms / `--text` / `--url` / `--load` / `--fn`) | ✅ | polled |

## State

| Command | Status | Notes |
| --- | --- | --- |
| `cookies` / `cookies set` / `cookies clear` | ✅ | `Network.getAllCookies` / `setCookie` / `Storage.clearDataForOrigin` |
| `storage local` / `session` (get/set/clear) | ✅ | HTTP origin only; opaque (`data:`) origins return empty |
| `set offline` | ✅ | `Network.emulateNetworkConditions` — real |
| `set headers` | ✅ | `Network.setExtraHTTPHeaders` — real |
| `set viewport` | 🟡 | `Emulation.setDeviceMetricsOverride` — ack only, effect unverified |
| `set device` / `geo` / `media` / `credentials` | 🟡 | ack only; no real effect (engine lacks the override) |
| `state save` / `load` / `list` / ... | ✅ | cookie + localStorage based; works on HTTP origins |
| `addinitscript` / `removeinitscript` / `--init-script` | ❌ | `Page.addScriptToEvaluateOnNewDocument` is ack-only and **not executed** on Starfish |

## Network

| Command | Status | Notes |
| --- | --- | --- |
| `network requests` / `network request <id>` | ✅ | Real CDP Network events on the HTTP loader path (document + subresources) |
| `network har start` / `stop` | 🟡 | Records tracked requests; HTTP-path only |
| `network route` / `unroute` | 🟡 | `Fetch.enable` acks; `--abort` (setBlockedURLs) works, fulfill/continue mocking is best-effort/unverified |

## Tabs, windows, frames

| Command | Status | Notes |
| --- | --- | --- |
| `tab` / `tab new` / `tab <id>` / `tab close` | 🟡 | Targets spawn and switch on the headless build, but tabs share one WebView — **JS contexts are not isolated** and console stays on the initial tab |
| `window new` | 🟡 | Same shared-WebView caveat |
| `frame main` | ✅ | |
| `frame <sel>` | 🟡 | iframe target switching unverified on the single WebView |

## Dialogs

| Command | Status | Notes |
| --- | --- | --- |
| `dialog status` | ✅ | |
| `dialog accept` / `dismiss` | 🟡 | `javascriptDialogOpening`/`Closed` events fire, but `handleJavaScriptDialog` is **ack-only**: the page already resumed with the default value before the handler runs, so `accept`/prompt text cannot change what the page saw |

## Diff

| Command | Status | Notes |
| --- | --- | --- |
| `diff snapshot` / `diff url` | ✅ | accessibility-tree based |
| `diff screenshot` | 🟡 | pixel diff over blank images — not meaningful on headless |

## Debug & profiling

| Command | Status | Notes |
| --- | --- | --- |
| `console` / `errors` | ✅ | |
| `highlight` | ✅ | no visible overlay (blank pixels) |
| `trace start` / `stop` | ❌ | `Tracing.*` acks but emits no `tracingComplete` / data |
| `profiler start` / `stop` | ❌ | Profiler is not wired to CDP |
| `inspect` | ❌ | Opens Chrome DevTools — not applicable to Starfish |

## React & Web Vitals

| Command | Status | Notes |
| --- | --- | --- |
| `react tree` / `inspect` / `renders` / `suspense` | ❌ | The React DevTools hook cannot be installed (init scripts are not executed) |
| `vitals` | 🟡 | Returns the summary shape, but paint/LCP/CLS/INP metrics are empty (no PerformanceObserver timing) |
| `pushstate` | ✅ | framework-agnostic |

## Clipboard & streaming

| Command | Status | Notes |
| --- | --- | --- |
| `clipboard read` / `write` / `copy` / `paste` | ❌ | `navigator.clipboard` is undefined in the engine |
| `stream enable` / `disable` / `status` | 🟡 | The WebSocket server runs and reports state, but screencast frames are blank/unsupported |

## Chat & dashboard

| Command | Status | Notes |
| --- | --- | --- |
| `chat` | ✅ | LLM-driven; uses the supported commands above |
| `dashboard` | 🟡 | Runs; the live viewport is blank (Mock graphics) while the activity feed works |

---

The underlying per-domain CDP contract (what Starfish's server actually returns
for each method) is documented in `starfish-cdp-skill/ANALYSIS.md`.
