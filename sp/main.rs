extern crate sp_derive;

mod instance;
mod opts;
mod value_parser;

#[cfg(test)]
mod tests;

use instance::SpawnerOptions;
use opts::CmdLineOptions;
use std::env;

fn main() {
    let argv: Vec<String> = env::args().collect();
    if argv.len() == 1 {
        print!("{}", SpawnerOptions::help());
    } else {
        let mut opts = SpawnerOptions::default();
        if let Err(e) = opts.parse(&argv[1..]) {
            print!("{}", e);
        }
    }
}
