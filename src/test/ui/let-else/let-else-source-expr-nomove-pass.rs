// run-pass
// issue #89688

#![feature(let_else)]

fn example_if_let(value: Option<String>) {
    let Some(inner) = value else {
        println!("other: {:?}", value); // OK
        return;
    };
    println!("inner: {}", inner);
}

fn main() {
    example_if_let(Some("foo".into()));
    example_if_let(None);
}
