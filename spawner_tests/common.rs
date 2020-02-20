use rand::distributions::Alphanumeric;
use rand::{thread_rng, Rng};

use std::fs;
use std::io::prelude::*;
use std::iter;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::Duration;

use cfg_if::cfg_if;

pub struct TmpDir {
    dir: PathBuf,
}

pub const MEM_ERR: u64 = 2 * 1024 * 1024; // 2MB
pub const TIME_ERR: f64 = 0.15; // 150 ms

macro_rules! target_dir {
    ($s:expr) => {
        concat!(env!("CARGO_MANIFEST_DIR"), "/../target/debug/", $s);
    };
    () => {
        target_dir!("");
    };
}

cfg_if! {
    if #[cfg(windows)] {
        pub const APP: &str = target_dir!("app.exe");
        pub const SP: &str = target_dir!("sp.exe");
    } else if #[cfg(unix)] {
        pub const APP: &str = target_dir!("app");
        pub const SP: &str = target_dir!("sp");
    }
}

impl TmpDir {
    pub fn new() -> Self {
        let mut rng = thread_rng();
        let name: String = iter::repeat(())
            .map(|()| rng.sample(Alphanumeric))
            .take(7)
            .collect();

        let dir = PathBuf::from(name);
        fs::create_dir(dir.as_path()).unwrap();

        Self {
            dir: dir.canonicalize().unwrap(),
        }
    }

    pub fn file<P: AsRef<Path>>(&self, filename: P) -> String {
        let mut path = self.dir.clone();
        path.push(filename);
        path.to_str().unwrap().to_string()
    }
}

impl Drop for TmpDir {
    fn drop(&mut self) {
        // The directory might be locked by another programm.
        for _ in 0..5000 {
            match fs::remove_dir_all(self.dir.as_path()) {
                Err(_) => thread::sleep(Duration::from_millis(1)),
                Ok(_) => break,
            }
        }
    }
}

#[macro_export]
macro_rules! assert_approx_eq {
    ($a:expr, $b:expr, $diff:expr) => {{
        match (&$a, &$b, &$diff) {
            (a_val, b_val, diff_val) => {
                if (*a_val < (*b_val - *diff_val)) || (*a_val > (*b_val + *diff_val)) {
                    panic!(
                        "assertion failed: |a - b| < diff \
                         a: `{:?}`, b: `{:?}`, diff: `{:?}`",
                        a_val, b_val, diff_val
                    )
                }
            }
        }
    }};
}

pub fn read_all<P: AsRef<Path>>(path: P) -> String {
    let mut result = String::new();
    let _ = fs::File::open(path).unwrap().read_to_string(&mut result);
    result
}

pub fn write_all<P, S>(filename: P, data: S)
where
    P: AsRef<Path>,
    S: AsRef<str>,
{
    let mut file = fs::File::create(filename).unwrap();
    let _ = write!(file, "{}", data.as_ref());
}
