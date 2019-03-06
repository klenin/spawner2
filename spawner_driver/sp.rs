extern crate spawner_driver;

fn main() {
    if let Err(e) = spawner_driver::run(std::env::args().skip(1)) {
        eprintln!("{}", e);
    }
}