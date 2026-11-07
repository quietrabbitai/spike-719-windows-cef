# RUN_STEPS_720 (macOS, items.id=720): Jason drives, the code builds and instruments

THROWAWAY spike. Short scope: the decisive check is whether a transparent WKWebView composites over the pane surface,
so HTML modals draw over and dim the panes. Everything below is done by you at the Mac's own console.
The code requests no macOS permission (no Accessibility, no Screen Recording, no Keychain). If macOS ever shows a
permission dialog, **click Don't Allow and tell me**; that is a finding.

## Rules
- Use the Mac's own screen and keyboard. **Never over Sunshine / Screen Sharing** (it changes compositing).
- **Throwaway accounts only** for Claude, ChatGPT and Google. I never type credentials; you do.
- Quit with **Cmd+Q**. (I tested only an app-driven exit, which is clean; Cmd+Q itself is untested by me, so tell me if it crashes.) Do not `kill` the process: SIGTERM reproducibly crashes the app in
  `cef::shutdown` (probable cause: Chromium's own signal handling; not proven; recorded in the findings).
- Recording: Cmd+Shift+5 (macOS's own recorder; you grant that yourself, I don't ask). One recording per mode is fine;
  say the check number out loud or note timestamps.
- After each mode, keep any crash report (also if there was no popup): see "Crash reports".

## Get it running (already built)
```bash
cd ~/dev/spike-719-windows-cef
tools/bundle-mac.sh adhoc          # only if target/bundle/spike720.app is missing or stale
```
Run each mode from Terminal (the window opens on the main display, 1920x1080 at 1x):
```bash
APP=~/dev/spike-719-windows-cef/target/bundle/spike720.app/Contents/MacOS/spike720
$APP --mode=b-gpu-below --profile=$HOME/dev/spike-719-windows-cef/results/jason-b-gpu-below
$APP --mode=b-sw-below  --profile=$HOME/dev/spike-719-windows-cef/results/jason-b-sw-below
$APP --mode=a           --profile=$HOME/dev/spike-719-windows-cef/results/jason-a
```
Each profile folder gets `env.json`, `spike.log`, `events.csv`, `frames.csv`, `cef.log` and the CEF profile (it holds
your test-account sessions; delete `results/jason-*` when finished).

## Checks, in this order, in EACH mode (about 10 minutes per mode)
Use the toolbar **Mark** dropdown + **Log mark** before each step so the logs line up.

1. **Plain look.** Two panes show the local test page. Note anything odd: black pane, flicker, wrong position, seams.
2. **DECISIVE: HTML over panes.** Click **Open modal**: does the dialog draw above BOTH panes with the panes dimmed
   under the scrim, no seams or halos at the pane edges? Repeat for **Menu**, for hovering **Hover: tall tooltip**
   and for hovering the rail icons. Does the spinner in a pane keep animating under the scrim?
3. **Move / resize / maximize.** Drag the window by the title bar for ~5 s; drag every edge and corner; green button
   (full screen) and back. Do the panes lag behind, tear, show stretched or old-size content, or go black?
4. **Typing and editing.** Type in the text boxes, the textarea and the contenteditable; open the `<select>` and pick
   an option; select the sentence and copy with Cmd+C and with the right-click menu; paste (Cmd+V) into the other pane.
   Note the cursor shape over the text box and over buttons. If you have a CJK input method, compose and commit.
5. **Scroll.** Mouse wheel and trackpad (with inertia) in the long list. Smooth? Any stutter? (page-side fps line shows.)
6. **Popup.** Click both `window.open` buttons. Does a popup appear, is it usable, does "Finish and close" reach the
   opener? (`a`: CEF's own popup window is expected; B modes: host-drawn popup in the middle of the window.)
7. **Cookies.** In pane 1 and pane 2 click **set cookie** then **show cookies**. Quit (Cmd+Q), start the same command
   again: do the cookies survive? (Alloy runtime: separate jar per pane expected.)
8. **Retina (instrumentation only).** Run `tools/out/vdisplay` in one Terminal tab (creates a throwaway virtual 2x
   display, nothing installed), then start the mode with `--screen=virtual2x` in another. Click a few spots and
   type. **No visual verdict is wanted**; I read the log (backing scale, IOSurface and drawable sizes, hit coordinates).
   Stop `vdisplay` with Ctrl-C afterwards. Real-panel Retina stays "not tested".
9. **Real logins, LAST.** In the pane dropdowns choose https://claude.ai/ then https://chatgpt.com/ and sign in with a
   throwaway account (note any Cloudflare/Turnstile check). **Gemini is the very last step of the mode**:
   https://gemini.google.com/ with a throwaway Google account. Write down any "This browser or app may not be secure"
   block word for word, or that sign-in worked. Then Cmd+Q.

## Crash reports (after each mode, even without a popup)
```bash
mkdir -p ~/dev/spike-719-windows-cef/results/crash/<mode>
cp ~/Library/Logs/DiagnosticReports/spike720* ~/dev/spike-719-windows-cef/results/crash/<mode>/ 2>/dev/null; ls ~/dev/spike-719-windows-cef/results/crash/<mode>
```
(`<mode>` = a, b-sw-below or b-gpu-below. Reports from before your session are mine; tell me the time you started.)

## Sandbox (I can do this, or you can)
While a mode runs: `tools/check-sandbox.sh $(pgrep -n -f 'spike720 --mode')` prints the per-process table with controls.

## What to send back
Your filled `RESULTS_720.md` (observed column and result), the recordings, and anything odd. Do not send the `cef`
subfolders of the profiles (they hold test-account sessions).
