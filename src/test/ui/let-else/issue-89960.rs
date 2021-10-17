// This had an ICE, see issue #89960

#![feature(let_else)]
#![deny(unused_variables)]

fn main() {
    let Some(ref mut meow) = Some(()) else { return };
    //~^ ERROR unused variable: `meow`
}
