# RESULTS_720 (items.id=720, macOS)

Result column: PASS / COMPROMISE (describe what the user sees) / FAIL / not tested. "Same as Linux" is YOUR judgement
against the Linux build: yes / no (one line why). Do not leave a cell blank: write `not tested` and why.

## Machine
| Field | Value |
|---|---|
| Mac | Mac mini (M1, arm64), 8 GB, `sysctl hw.model` in env.json |
| macOS | 27.2 (build 26B123) |
| Display | one VA2446, 1920x1080, backingScaleFactor 1.0 (no Retina panel attached) |
| Virtual 2x display | tools/vdisplay (private CGVirtualDisplay), instrumentation only, compromise |
| CEF | 151.1.0+151.3.12 (Chromium 151.0.7922.47), "minimal" macOS arm64 distribution |
| Tauri / tao / wry | 2.11.2 / 0.35.3 / 0.55.1 |
| wgpu | **30.0.1 in this spike's Cargo.lock; production pins 30.0.0** (not tested against 30.0.0) |
| Rust | 1.97.1 (pinned) |
| Branch | `mac-720` (local only, nothing pushed) |

## Measured by the code, no input needed (filled in by Claude)
See FINDINGS_720.md (written after your session). Raw files are in `results/run-*`.

## Your table (one block per mode: a, b-sw-below, b-gpu-below)
| # | Check | Observed (what you see) | Recording / timestamp | Result | Same as Linux (yes/no, why) |
|---|---|---|---|---|---|
| 1 | Plain look (panes visible, no seams/black/flicker) | | | | |
| 2 | **DECISIVE**: modal dims both panes; menu and tooltips draw over panes; spinner keeps running underneath | | | | |
| 3 | Move / resize / maximize: no lag, tearing, stretched or black panes | | | | |
| 4 | Typing, select dropdown, Cmd+C/V, right-click menu, cursor shapes, IME | | | | |
| 5 | Scroll: wheel and trackpad inertia | | | | |
| 6 | Popup (`window.open`) and "Finish and close" | | | | |
| 7 | Cookies: per-pane jars, survive restart | | | | |
| 8 | Retina: virtual 2x, instrumentation only (no visual verdict) | not judged | n/a | instrumented | n/a |
| 9a | Claude.ai sign-in (throwaway) | | | | |
| 9b | ChatGPT sign-in (throwaway) | | | | |
| 9c | **Gemini sign-in (throwaway Google), run LAST**: exact block text or success | | | | |
| 10 | Crash report kept in `results/crash/<mode>`? (file names, or "none") | | | | |

## Compromises (each is confirmed or struck by the results; nothing is worked around silently)
| # | Compromise | Status |
|---|---|---|
| 1 | `tauri/macos-private-api` + `macOSPrivateApi` needed for a transparent WKWebView (private API; Mac App Store blocker). Enabled for all platforms in Cargo.toml/tauri.conf.json because tauri-build checks only `[dependencies]` | confirmed (build fails without it) |
| 2 | NSApp must answer CefAppProtocol: methods added to NSApplication at runtime so tao's TaoApp inherits them; `sendEvent:` swizzled. `class_conformsToProtocol` says false for the KVO-wrapped TaoApp while `[NSApp conformsToProtocol:]` and `respondsToSelector:` say true | confirmed |
| 3 | External message pump on the main thread (no multi-threaded loop on macOS) | confirmed |
| 4 | Mode `a`: `hitTest:` added to wry's WKWebView class so mouse hits pass through to the CEF views | confirmed; an isa swap onto a subclass crashed (KVO), see findings |
| 5 | `--use-mock-keychain` (avoids a Keychain permission prompt; cookies not Keychain-encrypted) | confirmed |
| 6 | Virtual 2x display is not a real Retina panel | confirmed |
| 7 | Ad-hoc signature only: Chromium logs `process_requirement.cc: Unable to derive validation category` at start; no Developer ID, no notarization | confirmed |
| 8 | SIGTERM crashes the app in `cef::shutdown` (SIGTRAP inside the CEF framework, every time; cause inferred, not proven: probably Chromium's own SIGTERM handling starting a shutdown that our `cef::shutdown` then repeats); an app-driven exit (Tauri `exit`, the same `RunEvent::Exit` that Cmd+Q should reach) is clean (status 0, no crash report); Cmd+Q itself is untested by me, step 7 of the run sheet covers it | confirmed |
