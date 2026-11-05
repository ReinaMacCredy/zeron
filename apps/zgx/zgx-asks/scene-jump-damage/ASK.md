# Ask: a node removed in the flush that jumps a scroll past the view leaves its pixels

**Priority: high.** Users see it. With partial repaint on, a transcript can show
an off-screen reply's old pixels over its last rows, and they stay there until
something else repaints that place.

All at zgx `bebc9b34`. `zeronsh/zgx` main `07ebcf013` has the same opening in
`resolve_moves`, read through the GitHub API and not run.

**Today.** In one flush a scroll moves the content by at least the view's height,
and an effect on the offset removes a node that has moved off screen:

1. `Scene::remove_as` damages the subtree where it is now. `damage_painted`
   intersects that with the clip, so nothing is recorded.
2. `resolve_moves` finds the move can't be copied and damages everything the
   content painted, at both positions. That walks only the nodes still mounted,
   and `damage_before_move` has nothing to translate back.
3. The old place of the removed node is never repainted.

A box laid out smaller after such a jump leaves the same stale pixels.
`a_moved_layer_repaints_what_changed_after_the_move_where_it_was` uses a 30 pt
move, where the node stays in the clip, so it doesn't reach this case.

**Repro.** `repro.rs` here goes in `crates/zgx/tests/`. A scroll view 200 tall,
a lane at `y=100`, a jump to 600:

```text
removed_after_a_jump_past_the_view   damage=[]                  uncovered=960   FAIL
shrunk_after_a_jump_past_the_view    damage=[0,100 40x16]       uncovered=1280  FAIL
removed_before_a_jump_past_the_view  damage=[0,100 60x16]       uncovered=0     ok
removed_one_flush_after_the_jump     first=[0,100 60x16]        uncovered=0     ok
removed_after_a_small_move           damage holds both places                   ok
```

**Ask.** A jump past the view repaints all the view shows, the path `again`
already takes, in `resolve_moves`:

```rust
let jumped = self
    .world_clip(id)
    .is_some_and(|clip| dx.abs() >= clip.width || dy.abs() >= clip.height);
let again = again || jumped;
```

With it on a scratch copy of `bebc9b34`: the repro passes, `cargo test -p zgx --lib
--test lists --test compose --test paint --test layout` 866 passed,
`cargo test -p zgx-gpu --lib` 122 passed. Not run in the app, and the cost of the
jump frame was not measured.

**Zeron meanwhile.** `ds/markdown.luau`'s pass in the same flush as the scroll only
builds runs, and its tick releases them a frame later. That covers this one caller:
another removal or a shrink in that flush stays exposed.
