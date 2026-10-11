# RUN_STEPS (STAGE 2: modes A and B)

Stage 1 (mode A) is done. This drop adds Approach B (windowless CEF drawn through wgpu/D3D12 onto native child
windows). Run the `b-*` modes with the same checklist as mode A, from a NORMAL (non-admin) PowerShell.

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


## Stage 2: the B modes
Four modes, run each with the same checklist (use the Mark dropdown before each step):
```
.\spike719.exe --mode=b-sw-above    # software upload (what Linux ships), pane windows ABOVE the webview, native input
.\spike719.exe --mode=b-sw-below    # software upload, pane windows BELOW a transparent webview, DOM hit-layer input
.\spike719.exe --mode=b-gpu-above   # D3D11 shared-texture import into wgpu/D3D12, above
.\spike719.exe --mode=b-gpu-below   # same, below
```
What is different from mode A, and what to look for:
- **Modal / menu / tooltips**: in the `below` modes the HTML must draw above the panes with the panes dimmed under the
  backdrop. In the `above` modes the pane windows are on top of the webview again (expect the same airspace problem as A).
- **Black pane in the gpu modes**: if a gpu mode shows black or frozen panes, that is the silent-fallback failure the
  Linux build hit. Look at `frames.csv` (`import_ok` / `import_err`) and tell me.
- **Right-click** in a pane: a Win32 menu (Copy / Cut / Paste / Select all). Copy uses the selection-cache path
  (selected text written to the clipboard by the host). Paste into the other pane's text box.
- **Popups**: the two `window.open` buttons open a host-drawn popup in the middle of the window (red frame in the
  `below` modes). Use "Finish and close", or the toolbar "Close popup" button.
- **`<select>` dropdown** in a pane: does the option list appear and can you pick an option?
- **Keyboard / IME**: type in the text box; with a CJK input method, compose and commit (below modes forward IME
  from the page; above modes rely on Windows' default handling - note what you see).
- **Cursor**: does it change to a text cursor over the text box and a hand over buttons?
- **Cookies**: B modes use the Chrome runtime (as Linux does) and therefore share one cookie jar across both panes.
  Try `--runtime=alloy` once to see whether per-pane jars work there (set a cookie in pane 1, show cookies in pane 2).
- **Resize/move/DPI**: same as mode A; also change Windows display scaling (100/125/150/200%) while running one
  mode, and say whether text stays sharp and the panes stay aligned.
- **Soak**: leave one mode running for 30 minutes (after "start: soak" mark), then check it did not freeze or leak.

Extra files in `%TEMP%\qr-spike-719\`: `frames.csv` (per-second CEF paint rate/gaps and present times per pane).
