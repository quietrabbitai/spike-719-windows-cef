# spike-719-windows-cef

THROWAWAY spike for Quiet Rabbit items.id=719 (Windows Cloud Chat pane host). It is a test bed, not a product.
It is never merged into quietrabbit-core and this repository is deleted at closure.

It hosts two CEF panes inside one Tauri 2 window on Windows so two approaches can be compared against the
Linux build's quality (decisions.id=874, 876):

- **A**: windowed CEF children parented to the Tauri window's HWND.
- **B**: windowless (off-screen) CEF drawn through wgpu onto a native child surface, above or below a
  transparent webview. (Stage 2; not in this build.)

## What it contacts
Only: a loopback test server on `127.0.0.1` (random port, never reachable from the network), and, when you
choose them in the toolbar, `https://claude.ai`, `https://chatgpt.com`, `https://gemini.google.com`.
No telemetry. Chromium background networking, component updates, sync and pings are switched off. The
Chromium sandbox is ON.

## What it writes
Everything goes under `%TEMP%\qr-spike-719\` (or `--profile=<dir>`): `env.json`, `spike.log`, `events.csv`,
`cpu.csv`, `procs.csv`, `cef.log`, plus the CEF and WebView2 profile folders. It never touches your browsers'
data. Delete that folder when you are done (see RUN_STEPS.md).

See RUN_STEPS.md and RESULTS_TEMPLATE.md.
