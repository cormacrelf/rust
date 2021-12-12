// check-pass
// This had an ICE, see issue #89960

#![feature(let_else)]

fn main() {
    // (You can't use just `_` in any `ref`/`ref mut` pattern)
    let Some(ref mut _a) = Some(()) else { return };
}
