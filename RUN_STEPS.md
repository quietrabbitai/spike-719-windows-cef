# RUN_STEPS (STAGE 1: smoke drop, mode A only)

Goal of this drop: find out whether the binary even starts on your PC, and get the first observations of
Approach A. Modes `b-*` are not built yet (they exit with a message).

## Rules
- Run on the PC itself, on a monitor. **Never over Remote Desktop** (it changes compositing).
- Use **throwaway test accounts** for Claude, ChatGPT and especially Google. Do not log in with a real account.
- `spike719.exe` is CEF's own `bootstrap.exe` (it creates the Chromium sandbox, then loads `spike719.dll`); keep both together.
- No installer, no system changes. The exe is unsigned: SmartScreen/Defender may warn. Run from the unzipped folder.
- At the end: delete `%TEMP%\qr-spike-719\` and sign out of every test account (they are only inside that folder, so
  deleting it signs them out locally; also sign out on the provider side if you want to be thorough).

## Get the binary
1. GitHub repo `quietrabbitai/spike-719-windows-cef` -> Actions -> `windows-build` -> latest green run -> Artifacts ->
   `spike719-windows-x64` (needs a GitHub login). Download, unzip to `C:\spike719\` (all files stay together).
2. Check the hash: PowerShell `Get-FileHash C:\...\spike719-windows-x64.zip -Algorithm SHA256` must equal the line
   `SHA256 spike719-windows-x64.zip = ...` in the run's log (also in SHA256.txt).

## Run (stage 1)
```
cd C:\spike719
.\spike719.exe --mode=a                    # CEF on its own thread (what Linux does)
.\spike719.exe --mode=a --cef-loop=pump    # CEF pumped from the main thread; run this second
```
Mode A now raises the CEF windows above the WebView2 layer by default. For the comparison run once with
`.\spike719.exe --mode=a --a-zorder=keep` (creation order, no raise) and say whether the panes are visible.
Run from a NORMAL (non-admin) PowerShell: an elevated launch makes Chromium relaunch itself de-elevated.
Close the window between runs. If the window never appears or the app exits, copy `%TEMP%\qr-spike-719\spike.log`
and `cef.log` into the results; that is itself the finding.

### What to do in each run (about 10 minutes)
1. Window opens with two panes showing the local page. Note anything odd (black pane, flicker, wrong position).
2. In the window toolbar use **Mark** -> pick a label -> **Log mark** just before each step below so the logs line up.
3. **Move** the window by its title bar for ~5 s; **resize** by dragging each edge/corner; **maximize** and restore;
   **snap** with Win+Left / Win+Right. Watch the panes: do they lag behind the window, tear, or lose size?
4. **Modal**: click "Open modal". Does the HTML dialog draw above both panes, or are panes drawn over it? Same for
   "Menu" and for hovering the rail icon (tooltip).
5. In a pane: type in the text box, open the dropdown, scroll the long list, select and copy the sentence
   (Ctrl+C and right-click menu), paste it in the other pane's text box.
6. Click **set cookie = this pane** in pane 1 and in pane 2, then **show cookies** in both: they must differ.
   Close and rerun with the same command: do the cookies survive?
7. Click the two `window.open` buttons: does a popup appear, is it usable, does "Finish and close" reach the opener?
8. Pane 1 dropdown: choose https://claude.ai/ and sign in with a test account (Cloudflare check?). Repeat for
   ChatGPT and Gemini (Google: note any "this browser is not secure" block).
9. If a CJK input method is installed, type in the IME box; otherwise write "not installed".
10. Leave it idle for 2 minutes on the local page (look at `cpu.csv` afterwards).

### Optional measurement
GPU load while idle/active: in PowerShell
`typeperf "\GPU Engine(*)\Utilization Percentage" -sc 30` (built into Windows, nothing to install).
Recordings: Win+G (Xbox Game Bar) -> capture. It records the composed output and may miss tearing; a phone
slow-motion clip of the screen is the backup for criterion 2.

## Send back
Zip `%TEMP%\qr-spike-719\` (leave out the `cef` and `webview2` subfolders: they hold the test-account sessions)
plus your filled RESULTS_TEMPLATE.md and the recordings.
