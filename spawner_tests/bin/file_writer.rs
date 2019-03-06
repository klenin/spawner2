use std::env;
use std::fs::{remove_file, File};
use std::io::Write;

// Writes n kilobytes to file, where file=argv[1], n=argv[2].
fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 3 {
        return;
    }

    let filename = &args[1];
    let kb = match args[2].parse::<f64>() {
        Ok(x) => x as usize,
        Err(_) => return,
    };

    let _ = remove_file(filename);
    let mut file = File::create(filename).unwrap();
    let chunk: Vec<u8> = (0..1024).map(|_| b'1').collect();

    for _ in 0..kb {
        let _ = file.write(&chunk);
    }

    std::thread::sleep(std::time::Duration::from_millis(1000));
}
