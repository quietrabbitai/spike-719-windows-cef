# RESULTS_TEMPLATE (items.id=719)

Fill the machine block once per session (env.json has most of it; confirm it). Then one row per
(criterion x mode). Do not leave a cell blank: write `not tested` and why.

## Machine (per session)
| Field | Value |
|---|---|
| Date / test session | |
| PC GPU model | |
| GPU driver version | |
| Display count | |
| Per-display resolution and Windows scaling (%) | |
| Windows build | |
| Monitor connection (not Remote Desktop) | |
| Spike zip SHA-256 matched? | |

## Results
Result column: PASS / COMPROMISE (describe what the user sees) / FAIL / not tested.
"Feels the same as Linux" is YOUR judgement against the Linux build: yes / no (and one line why).

| # | Criterion | Mode | Observed | Measured value | Recording file | Result | Feels the same as Linux (yes/no) |
|---|---|---|---|---|---|---|---|
| 1 | One window; HTML modal/menu/tooltip draw above or around panes, no seams | a | | | | | |
| 2 | Move / resize / maximize / snap: no lag or tearing | a | | | | | |
| 3 | Per-monitor DPI 100/125/150/200%, mixed DPI | a | | | | | |
| 4 | Text sharpness and scroll smoothness in a real provider page | a | | page-side fps line | | | |
| 5 | Keyboard focus, shortcuts, IME | a | | | | | |
| 6 | Copy/paste in and out; right-click menu | a | | | | | |
| 7 | Popups and OAuth windows | a | | | | | |
| 8 | Two panes, isolated cookie jars, persistence across restart | a | | | | | |
| 9 | Real logins: Claude.ai (Turnstile), ChatGPT, Gemini (Google block?) | a | | | | | |
| 10 | Chromium sandbox enabled (see procs.csv: integrity/restricted/in_job per process) | a | | | | | |
| 11 | Idle CPU/GPU; 30-minute stability | a | | cpu.csv summary | | | |
| 12 | Any compromise found (listed, not worked around) | a | | | | | |

Add the same rows for each further mode (`a --cef-loop=pump`, and the `b-*` modes in stage 2).
