use std::env;
use std::thread;
use std::time::Duration;

// Sleeps for argv[1] seconds.
fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 2 {
        return;
    }

    thread::sleep(match args[1].parse::<f64>() {
        Ok(x) => Duration::from_millis((x * 1000.0) as u64),
        Err(_) => return,
    });
}
