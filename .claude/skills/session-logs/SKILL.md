---
name: session-logs
description: >
  Find where agent sessions spend time and money in this repository: indexes every Claude Code
  transcript (interactive and driver-launched) into metrics, reports waiting-on-human time, tool
  errors, permission blocks and slow tools, and points at the raw transcript behind a row. Use
  when asked "why is this slow", "where did the time go", "what is blocking the loop", "review
  the driver runs", "audit sessions", or to look for process friction worth removing.
---

# Session logs

Transcripts already exist as `.jsonl` under `~/.claude/projects/`; this skill makes them
answerable without reading them whole.

```
python -m tools.plan.sessionlog index
python -m tools.plan.sessionlog report --since 7d
python -m tools.plan.sessionlog report --kind driver
python -m tools.plan.sessionlog show <session-id-prefix>
python -m tools.plan.driver status
```

The index lives in `~/.holy-blocker/sessions/index.jsonl` (override with `HB_LOG_HOME`). Rows are
metrics only: no prompt or response text. `report` also joins `~/.holy-blocker/driver/runs.jsonl`,
so a driver session shows its step id and outcome.

## Reading the report

- **waiting** is the time between the agent's last event and the next human prompt. A large share
  means the human is the scheduler: look for a gate that earlier decisions already settle.
- **active** is the sum of event gaps under ten minutes, excluding waits. It is the agent's working time.
- **tool errors / blocks** cluster on a tool that the permission mode or sandbox keeps refusing;
  `permission_denials` in a driver run is the same signal.
- **Slowest tools** is usually `Bash`; `bash_heads` in `show` names which command.

## Going from a row to the cause

`show` prints the row, including `path` to the transcript. Search that file narrowly instead of
reading it: `grep -n '"is_error": true'` for failures, or pull entries around a long wait by
timestamp. Raw driver output (the final result, cost, denials) is in
`~/.holy-blocker/driver/out/<session-id>.json`.

## Rules

- Report friction as a measured fact with the session ids behind it, then propose one change.
- Never copy transcript text into a commit, PR body, doc or memory; cite the session id instead.
- Nothing here leaves the machine.
