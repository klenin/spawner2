use std::env;

fn main() {
    for (name, val) in env::vars() {
        println!("{}={}", name, val);
    }
}