fn main() {
    match std::env::args().skip(1).next() {
        Some(file) => match std::fs::File::open(file) {
            Ok(_) => print!("ok"),
            Err(_) => print!("err"),
        },
        None => print!("none"),
    }
}
