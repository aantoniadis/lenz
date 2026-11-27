# Measure project-tab performance against the saved baseline

Status: ready-for-agent

AGENTS.md requires release measurements for performance-sensitive changes. Not yet run for this work:

- Single-project first paint, interaction and settled memory in both adapters against a baseline from the base
  commit (the single-project path adds one host element in the GUI and one projects check per draw in the TUI).
- Side by Side with 2 and 4 projects: frame interval while scrolling one column, RSS per added project, and the TUI's
  per-frame off-screen copy cost at large terminal sizes.
- Repeated open/close of project tabs reaching a stable memory range.
