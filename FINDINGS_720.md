# FINDINGS_720 (items.id=720: macOS Cloud Chat pane host spike)

THROWAWAY spike, branch `mac-720`, never merged into quietrabbit-core. Short scope (Jason, 2026-10-11). No production
design here. Raw logs, crash reports and recordings are in the local `results/` folder (gitignored, not in the repo).

## What was built and tested
Mac mini M1 (arm64), 8 GB, macOS 27.2, one 1920x1080 display at 1x (VA2446), Command Line Tools only (no Xcode needed).
CEF 151.1.0+151.3.12 (Chromium 151.0.7922.47), tauri 2.11.2, tao 0.35.3, wry 0.55.1, Rust 1.97.1.
**wgpu: this spike builds with 30.0.1 (its Cargo.lock); production pins 30.0.0. Nothing was run against 30.0.0.**

| Mode | What it is |
|---|---|
| `a` | windowed CEF child NSViews (`set_as_child`) below a transparent WKWebView |
| `b-sw-below` | windowless CEF, software `on_paint` upload, one wgpu/Metal NSView per pane, below the WKWebView (what Linux ships) |
| `b-gpu-below` | windowless CEF, IOSurface shared-texture import into wgpu/Metal, same placement |

The `above` placements, idle CPU/soak and mixed-DPI runs are outside the short scope: not built, not tested.

## Results per check (PASS / COMPROMISE / FAIL / not tested)
| Check | a | b-sw-below | b-gpu-below |
|---|---|---|---|
| **Decisive**: transparent WKWebView composites over the panes; HTML modal draws over and dims them, no seam | PASS (seen) | PASS (seen) | PASS (seen; modal, menu, tooltips, move/resize confirmed by Jason) |
| Click, type, select, Cmd+C/V, right-click menu, dropdown | **FAIL** with the WKWebView above (no mouse or key event reaches the page); works natively with the WKWebView hidden | PASS | PASS |
| Popup (`window.open`) | not tested | PASS (title strip fixed) | PASS (Jason and me) |
| Cookies: per-pane jars, survive restart | not tested | PASS | PASS (Jason) |
| Move and resize, from frames.csv | not measured (no host frame log) | one resize step: first correct-size frame 81-97 ms | move: 0 layout changes, 59-60 paints/s, worst gap 45 ms, 0 stretched; resize 48 steps: first correct-size frame median 20.9 ms (p95 33.3, max 35.0), 4-5 stretched frames over the drag |
| Scroll, identical real input, page-side updates per gesture | 20 in 0.32 s (63/s, worst gap 18 ms) with the WKWebView hidden | 3 (~12/s, worst gap 117-135 ms) | 3 (~12.8/s, worst gap 117-133 ms) |
| Retina (virtual 2x display, instrumentation only) | scale and placement logged | frame = view pixels = drawable (1126x1290 for a 563.2x645.1 pt pane); DIP-to-pixel for IPC and real clicks correct | same |
| Chromium sandbox ON (decisions.id=860) | not run in `a` | children sandboxed | children sandboxed (browser process is not, as on macOS generally) |
| Passkey / WebAuthn | not available (finding 7) | not available (same log lines) | not available (same log lines) |
| Signing | ad-hoc and hardened-runtime builds launch and sandbox; Gatekeeper rejects both (no Developer ID); notarization not tested | | |
| Gemini login (throwaway account) | Gemini loaded in pane 1 through the new start-up URL option (logged). Jason: **"The login said account was recovered."**; Google then offered a passkey, and accepting it said the passkey is not on this device; **"Sign in another way" with password + 2FA worked** (no "browser not secure" block reported). Passkeys cannot work in this build (finding 7). Quitting this run **crashed** (finding 5) | not run | **worked** (Jason: "check 1 worked"; no further detail such as block text was reported) |
| Real-input scroll, **mouse and keyboard (no trackpad available)** (Jason) | not run | Jason: scrolling the **whole page** was smooth (seemed smoother than before); the **list inside the pane** was smoother too but **still shows the line shift, the same with the mouse wheel and with keyboard scrolling** (cause not isolated). Page-side log: one 8.95 s gesture = 462 updates, 51.6/s, p95 gap 19 ms, worst 403 ms; six other gestures 6-22 updates/s with worst gaps 600-765 ms. The page does not tag the input device, so which gesture was wheel, scrollbar drag or keyboard is not known | not run |
| Trackpad scroll (inertia, phases) | **not tested: no trackpad on this Mac** | | |
| Real-panel Retina | **not tested** (no Retina display; the virtual display is a compromise) | | |
| IME / CJK input | not tested | | |

## Findings that matter
1. **B-below composites correctly and takes input.** In both B modes the transparent WKWebView draws over the Metal pane
   surfaces, and typing, selection, copy/paste, the native context menu, dropdowns, popups and cookies work. The IOSurface
   import works with `import_err = 0`; there is no silent fallback.
2. **Mode `a` composites correctly but loses input.** CEF's windowed views are layer-composited under the WKWebView and the
   modal dims them. With the WKWebView above them, even with a `hitTest:` passthrough that demonstrably routes the click to
   Chromium's `RenderWidgetHostViewCocoa` (window key, that view first responder), the page receives no mouse or key event.
   Control: with the WKWebView hidden, clicks and typing work natively. Why the stacked WKWebView breaks input is **not
   isolated**. cef-rs issue #456 (`set_as_child` SIGSEGV on arm64) is treated as **unresolved**: no SIGSEGV was seen in these
   runs on 151.1.0, which is not evidence it cannot happen.
3. **Scroll is coarse in B and was not root-caused.** With the same input, native scrolling yields about 63 updates/s and B
   about 12/s (instant jumps, no smooth animation). Separately, host paints stall about 250-450 ms at the start of a scroll
   gesture and at direction changes (60 -> ~30-37 paints/s); this does not depend on wheel rate and is the same in the
   software and GPU paths; presenting a frame takes 0.5-7 ms. Input above was synthetic wheel events. Jason's real-input run
   (`b-sw-below`, mouse and keyboard; **there is no trackpad on this Mac, so trackpad scrolling is untested**): scrolling the
   whole page was smooth (one 462-update gesture, 51.6/s, worst gap 403 ms); the list inside the pane was smoother but still
   shows the line shift, **the same with the mouse wheel and with keyboard scrolling** (Jason). The page does not record the input device, so the six coarse gestures in the log (6-22 updates/s, worst gaps
   600-765 ms) cannot be attributed to the wheel with certainty. (The profile folder is named `trackpad-b-sw-below` only
   because of the command I gave; it is not a trackpad run.) Because keyboard scrolling (key events, no wheel events) shows the same
   shift, a horizontal component of the wheel events is **not** the cause. Where the shift comes from (CEF's windowless
   text rendering of the scrolling list, or how my Metal layer presents it) is not isolated; whether it also appears in
   native mode `a` is untested.
4. **Key events through the host were wrong, and a fix was found.** Versus native input, the IPC path produced
   `event.key == "Unidentified"` for letters and Shift, **two keydowns per press, and no keyup**. Cause (by experiment): with
   `character == 0`, CEF on macOS cannot derive `key` and turns a KEYUP into a keydown. Sending the character on both the
   down and the up event gives one keydown, one keyup, and `key`/`code`/`keyCode` identical to native for letters, Shift,
   Enter, arrows and Backspace in both B modes; Cmd+C/V still work. Committed as `3e218e9` (`src/host_mac.rs`,
   `ui/index.html`, `ui/testpage.html`; the commit also contains the key experiment harness). Before the fix, providers that test `e.key` (for example Enter to
   send) would not have worked reliably.
5. **Shutdown:** SIGTERM crashes the app inside `cef::shutdown` every time (cause inferred, probably Chromium's own signal
   handling); an app-driven exit and the menu Quit were clean in the B modes. **One more crash, on Jason's run of mode `a`
   with the WKWebView hidden (quit from the menu after ~37 s):** SIGTRAP on the main thread inside `cef::shutdown` (same
   signature as the SIGTERM crashes). The log shows the 3 s wait for the windowed browsers to close timing out
   (`all browsers closed=false`), the browsers closing 214 ms later, and then no `cef::shutdown returned`. Windowed panes take
   ~3.2 s to close, so `cef::shutdown` may run while a close is still completing; not proven, and earlier mode `a` quits
   were clean, so it is intermittent. **It happened again on Jason's second mode `a` run** (Gemini loaded, ~230 s, quit with Ctrl+Q/Cmd+Q: Jason wrote "control q"
   and saw an error; the log shows the quit path ran and ended without `cef::shutdown returned`, matching a crash dialog
   and a new report): same SIGTRAP signature, same sequence. So both of Jason's mode `a` quits crashed, while my own short
   mode `a` runs (10-20 s, local pages) exited cleanly; what differs (run length, page content, how it was quit) is not
   isolated. Reports saved in `results/crash/check-run-20261106/` (local, gitignored); the crash count is now 18. The first runs also crashed from a class swap I made on a
   KVO-wrapped WKWebView (fixed).
6. **Not a pane problem but needed:** the content view runs under the title bar, so the page's layout area is the window's
   `contentLayoutRect` (768 pt of 800), not the full content view. The first geometry used the full view and every pane click
   landed ~30 pt off; fixed.
7. **Passkeys do not work in this build, in any mode.** CEF's log says the Touch ID authenticator is unavailable because the
   `keychain-access-group` entitlement is missing or incorrect (expected value `.org.chromium.Chromium.webauthn`), and the
   Bluetooth (phone) route is unavailable because the process has no Bluetooth metadata and was launched from Terminal. The
   spike is ad-hoc signed with no entitlements and run as a bare executable. Jason saw this as "passkey is not on this
   device" after Google offered a passkey; password + 2FA worked. What a properly signed production build needs for
   passkeys is **not tested** here.

## Compromises found (each is a compromise, none worked around silently)
1. `tauri/macos-private-api` + `macOSPrivateApi` for a transparent WKWebView: private API, a Mac App Store blocker (enabled
   for all platforms because tauri-build checks only `[dependencies]`).
2. NSApplication must answer `CefAppProtocol`: methods added to `NSApplication` at runtime (tao's `TaoApp` inherits them) and
   `sendEvent:` swizzled. `class_conformsToProtocol` is false on the KVO-wrapped class while `conformsToProtocol:` and
   `respondsToSelector:` are true. Production would need a tao change or a `tauri-runtime-cef`-style bootstrap.
3. External message pump on the main thread (macOS has no multi-threaded CEF loop); differs from Linux and Windows.
4. Mode `a`: `hitTest:` added to wry's WKWebView class (and it still does not give input).
5. `--use-mock-keychain` (avoids a Keychain prompt; cookies not Keychain-encrypted).
6. Virtual 2x display is not a real panel. 7. Ad-hoc signature only; Chromium logs a signature-validation error at start.
8. Key events need the character on both down and up events (finding 4), and DOM `code` to macOS key-code mapping lives in
   the host (`kvk`, ANSI layout only).
9. No passkeys: the build is ad-hoc signed with no `keychain-access-group` entitlement and run as a bare executable (finding 7).

## Reported, never deciding
Code: ~1,640 lines of new Rust for the macOS host (bmac 571, macapp 309, host_mac 607 incl. self-tests and experiments,
macenv 81, macmain 52, bins 21) plus ~170 lines of tools; shared files touched are in the commit diff stat. Bundle 335 MB
(CEF framework 315 MB). First full release build 2.5 min including the CEF distribution download; incremental builds ~26 s.
No OOM; swap was 0 before and about 17 MB after the session; peak memory was not measured.

## Not verified / open
- Gemini login: details beyond "worked" for `b-gpu-below`; whether the mode `a` crash depends on run length, page content or
  the quit method (Ctrl+Q versus Cmd+Q versus menu, which Jason should confirm); passkeys with a properly signed build.
- Trackpad scrolling (no trackpad on this Mac; production Macs have one); real Retina panel; notarization and Gatekeeper with a Developer ID; IME composition; mixed DPI; soak.
- The Windows build was not rebuilt or run on this Mac after the shared-file edits.
- Why the WKWebView blocks input to windowed CEF views in mode `a`; root cause of the scroll-start stall.
- Commits on `mac-720`, local only, nothing pushed: `4e17d38` (spike), `3a5205c` (drawable-size log), `3e218e9` (key fix).
  This note is committed separately (`2da039d`, correction `ffdc9bb`); the start-up URL option for mode `a` (`SPIKE720_PANE<i>_URL`)
  is not yet in any commit.
