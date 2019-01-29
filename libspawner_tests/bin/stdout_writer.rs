use std::env;

// Writes argv[1] to stdout argv[2] times.
fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 {
        return;
    }

    let n = match args[2].parse::<f64>() {
        Ok(x) => x as usize,
        Err(_) => return,
    };

    for _ in 0..n {
        print!("{}", args[1]);
    }
}
