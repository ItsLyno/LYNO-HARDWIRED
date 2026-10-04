# MO2 Cyberpunk 2077 plugin (vendored copy)

`game_cyberpunk2077.py` is copied unchanged from
[ModOrganizer2/modorganizer-basic_games](https://github.com/ModOrganizer2/modorganizer-basic_games/blob/master/games/game_cyberpunk2077.py),
commit `3bd9da97c159a1bc05fd21e81199e997ed16e0f6` (2026-07-07), plugin version 3.0.1.
License: MIT, see `LICENSE` in this folder.

It is here for reference only: the launcher does not run it. MO2 ships its own
copy in `plugins/basic_games/games/`, and the one in the build's base package
is what players actually run.

The parts the launcher depends on are summarized in
[`docs/cyberpunk-mo2.md`](../../cyberpunk-mo2.md). When upstream changes
(executable names, generated files, settings), update that page, the rules in
`crates/core/src/rules.rs` and this copy together.
