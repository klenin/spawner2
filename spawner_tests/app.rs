use std::alloc::{alloc, Layout};
use std::env;
use std::fs;
use std::io::*;
use std::process;
use std::ptr;
use std::str;
use std::thread;
use std::time::{Duration, Instant};

struct Parser<T: Iterator<Item = String>>(T);

impl<T> Parser<T>
where
    T: Iterator<Item = String>,
{
    fn next(&mut self) -> String {
        self.0.next().unwrap()
    }

    fn parse<F>(&mut self) -> F
    where
        F: str::FromStr,
        <F as str::FromStr>::Err: std::fmt::Debug,
    {
        self.next().parse::<F>().unwrap()
    }

    fn parse_flt_secs(&mut self) -> Duration {
        Duration::from_millis((self.parse::<f64>() * 1e3) as u64)
    }
}

fn loop_(dur: Duration) {
    let t = Instant::now();
    while (Instant::now() - t) < dur {
        for _ in 0..50000 {}
    }
}

fn alloc_(bytes: usize) {
    unsafe {
        let ptr: *mut u8 = alloc(Layout::from_size_align_unchecked(bytes, 1));
        for i in 0..bytes {
            *ptr.offset(i as isize) = 101;
        }
    }
}

fn fwrite(filename: String, kb: usize) {
    let _ = fs::remove_file(&filename);

    let mut file = fs::File::create(&filename).unwrap();
    let chunk: Vec<u8> = (0..1024).map(|_| b'1').collect();

    for _ in 0..kb {
        let _ = file.write(&chunk);
    }
}

fn pipe_loop() {
    let mut chunk = [0 as u8; 128];
    while let Ok(bytes) = stdin().read(&mut chunk) {
        if bytes == 0 {
            break;
        }
        let _ = stdout().write_all(&chunk[..bytes]);
        let _ = stderr().write_all(&chunk[..bytes]);
    }
}

fn wake_loop() {
    let mut line = String::new();
    let stdin = stdin();
    while let Ok(bytes) = stdin.lock().read_line(&mut line) {
        if bytes == 0 {
            break;
        }
        let hash_pos = line.find(|c| c == '#').unwrap();
        let agent = line[..hash_pos].parse::<u64>().unwrap();
        print!("{}W#\n", agent);
    }
}

fn main() {
    let mut p = Parser(std::env::args().skip(1));
    while let Some(arg) = p.0.next() {
        match arg.as_str() {
            "print_env" => env::vars().for_each(|(k, v)| println!("{}={}", k, v)),
            "abnormal_exit" => {
                let ptr: *const i32 = ptr::null();
                process::exit(unsafe { *ptr });
            }
            "loop" => loop_(p.parse_flt_secs()),
            "sleep" => thread::sleep(p.parse_flt_secs()),
            "alloc" => alloc_((p.parse::<f64>() * 1024.0 * 1024.0) as usize),
            "fwrite" => fwrite(p.next(), p.parse()),
            "pipe_loop" => pipe_loop(),
            "print_n" => {
                let s = p.next();
                (0..p.parse::<usize>()).for_each(|_| print!("{}", s));
            }
            "wake_loop" => wake_loop(),
            "try_open" => match fs::File::open(p.next()) {
                Ok(_) => print!("ok"),
                Err(_) => print!("err"),
            },
            "exec_rest" => {
                let _ = process::Command::new(p.next()).args(p.0).spawn();
                return;
            }
            "exec_rest_and_sleep" => {
                let _ = process::Command::new(p.next()).args(p.0).spawn();
                thread::sleep(Duration::from_secs(1));
                return;
            }
            _ => print!("{}", arg),
        }
    }
}
