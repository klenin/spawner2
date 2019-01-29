use std::env;
use std::io::{stdin, stdout, Read, Write};

// Reads argv[1] bytes from stdin and writes them into stdout.
fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 2 {
        return;
    }

    let n = match args[1].parse::<usize>() {
        Ok(x) => x,
        Err(_) => return,
    };

    let mut buf: Vec<u8> = Vec::new();
    buf.resize(n, 0);

    let _ = stdin().read_exact(&mut buf);
    let _ = stdout().write(&buf);
}
