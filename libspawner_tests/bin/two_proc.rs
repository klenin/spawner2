use std::env;
use std::process::Command;
use std::thread;

// Creates 2 processes.
fn main() {
    if env::args().count() == 1 {
        Command::new(env::current_exe().unwrap())
            .arg("arg")
            .spawn()
            .unwrap();
    }
    thread::sleep(std::time::Duration::from_secs(2));
}
