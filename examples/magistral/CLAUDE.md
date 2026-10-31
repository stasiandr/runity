# magistral

A railway deckbuilder on scrap — see README.md (in Russian) for the rules.

* The rules are `rules/` and do not depend on the engine. Change a rule
  there, then run `cargo test -p magistral-rules` and the depth report
  (`cargo run --release -p magistral-rules --bin depth`). A change that
  makes the ladder flatter (search no longer clearly beating greedy) needs
  a reason.
* Numbers go in `content/magistral/core/rules.ron`, not in code.
* The window is one HTML screen (`content/magistral/ui/screens/`), filled
  by `src/front.rs` (`Front::view`) and read back as `Press`es in
  `src/table.rs`. `cargo test` checks the screen reads and draws
  `target/shots/game.png` on a machine with a GPU (lavapipe will do).
