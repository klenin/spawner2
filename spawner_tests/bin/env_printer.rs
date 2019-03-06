fn main() {
    for (name, val) in std::env::vars() {
        println!("{}={}", name, val);
    }
}
