use std::alloc::{alloc_zeroed, Layout};
use std::env;
use std::f64;
use std::thread;
use std::time::Duration;

// Allocates n bytes, where n=argv[1].
fn main() {
    let args: Vec<_> = env::args().collect();
    if args.len() != 2 {
        return;
    }

    let mb = match args[1].parse::<f64>() {
        Err(_) => return,
        Ok(x) => x,
    };

    let bytes = (mb * 1024.0 * 1024.0) as usize;
    let _mem = unsafe { alloc_zeroed(Layout::from_size_align_unchecked(bytes, 2)) };

    thread::sleep(Duration::from_millis(1000));
}
