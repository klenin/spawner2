#![allow(dead_code)]

mod opts;

use opts::Matches;
use opts::Options;
use std::env;

macro_rules! opt_error {
    ($desc:expr, $opt:expr, $val:expr) => {
        format!("invalid value '{}' in '{}={}'. {}", $val, $opt, $val, $desc)
    };
}

/// Returns default_value if an option was not matched,
/// otherwise returns parsed value or error message
fn parse_bool_value(matches: &Matches, opt: &str, default_value: bool) -> Result<bool, String> {
    let val = match matches.get(opt) {
        Some(v) => v,
        None => return Ok(default_value),
    };
    let result = match val.chars().next() {
        Some(c) => match c {
            '1' => Some(true),
            '0' => Some(false),
            _ => None,
        },
        None => None,
    };
    if result.is_some() && val.len() == 1 {
        Ok(result.unwrap())
    } else {
        Err(opt_error!("Value must be either 0 or 1", opt, val))
    }
}

/// Splits num into (number, suffix)
fn split_number<'a>(num: &'a str) -> (&'a str, &'a str) {
    let mut suffix_len = 0;
    for c in num.chars().rev() {
        if c.is_digit(10) {
            break;
        } else {
            suffix_len += 1;
        }
    }
    let len = num.len();
    (&num[0..len - suffix_len], &num[len - suffix_len..len])
}

/// Returns unit multiplier that is relative to seconds or megabytes
fn parse_unit(c: char, is_time_unit: bool) -> Option<f64> {
    if is_time_unit {
        match c {
            's' => Some(1.0),
            'm' => Some(60.0),
            'h' => Some(3600.0),
            'd' => Some(86400.0),
            _ => None,
        }
    } else {
        match c {
            'b' => Some(1.0 / (1024.0 * 1024.0 * 8.0)),
            'B' => Some(1.0 / (1024.0 * 1024.0)),
            _ => None,
        }
    }
}

/// Returns degree multiplier that is relative to seconds or megabytes
fn parse_degree(num: f64, c: char, is_time_degree: bool) -> Option<f64> {
    assert!(num != 0.0, "num is zero");
    match c {
        'd' => return Some(1e-1),
        'c' => return Some(1e-2),
        'm' => return Some(1e-3),
        'u' => return Some(1e-6),
        'n' => return Some(1e-9),
        'p' => return Some(1e-12),
        'f' => return Some(1e-15),
        _ => {}
    }
    if is_time_degree {
        match c {
            '%' => Some(100.0 / num),
            'k' => Some(1e3),
            'M' => Some(1e6),
            'G' => Some(1e9),
            'T' => Some(1e12),
            'P' => Some(1e15),
            _ => None,
        }
    } else {
        match c {
            'k' => Some(1.0 / 1024.0),
            'M' => Some(1.0),
            'G' => Some(1024.0),
            'T' => Some(1024.0 * 1024.0),
            'P' => Some(1024.0 * 1024.0 * 1024.0),
            _ => None,
        }
    }
}

/// Returns suffix multiplier (that is relative to seconds or megabytes)
/// or error message
fn parse_time_or_memory_suffix(
    num: f64,
    suffix: &str,
    is_time_suffix: bool,
) -> Result<f64, String> {
    let mut chars = suffix.chars();
    match suffix.len() {
        0 => return Ok(1.0),
        1 => {
            let c = chars.next().unwrap();
            if let Some(v) = parse_degree(num, c, is_time_suffix) {
                return Ok(v);
            }
            if let Some(v) = parse_unit(c, is_time_suffix) {
                return Ok(v);
            }
        }
        2 => {
            let degree = parse_degree(num, chars.next().unwrap(), is_time_suffix);
            let unit = parse_unit(chars.next().unwrap(), is_time_suffix);
            if unit.is_some() && degree.is_some() {
                return Ok(unit.unwrap() * degree.unwrap());
            }
        }
        _ => {}
    }
    Err(String::from("Invalid unit"))
}

/// Returns default_value if an option was not matched,
/// otherwise returns time or memory value (in seconds or megabytes) or error message
fn parse_time_or_memory_value(
    matches: &Matches,
    opt: &str,
    default_value: f64,
    is_time_value: bool,
) -> Result<f64, String> {
    let val = match matches.get(opt) {
        Some(v) => v,
        None => return Ok(default_value),
    };
    let (num_str, suffix) = split_number(val.as_str());
    if num_str.is_empty() {
        return Err(opt_error!("Expected number", opt, val));
    }

    fn round6(v: f64) -> f64 {
        (v * 1e6).round() / 1e6
    }

    let num = match num_str.parse::<f64>() {
        Ok(v) => round6(v),
        Err(e) => return Err(opt_error!(e.to_string(), opt, val)),
    };
    if num == 0.0 {
        return if is_time_value {
            Err(opt_error!("Time cannot be set to 0", opt, val))
        } else {
            Err(opt_error!("Memory cannot be set to 0", opt, val))
        };
    }
    match parse_time_or_memory_suffix(num, suffix, is_time_value) {
        Ok(v) => Ok(round6(num * v)),
        Err(e) => Err(opt_error!(e, opt, val)),
    }
}

/// Returns default_value if an option was not matched,
/// otherwise returns time value (in seconds) or error message
fn parse_time_value(matches: &Matches, opt: &str, default_value: f64) -> Result<f64, String> {
    parse_time_or_memory_value(matches, opt, default_value, true)
}

/// Returns default_value if an option was not matched,
/// otherwise returns memory value (in megabytes) or error message
fn parse_memory_value(matches: &Matches, opt: &str, default_value: f64) -> Result<f64, String> {
    parse_time_or_memory_value(matches, opt, default_value, false)
}

fn main() {
    let args: Vec<_> = env::args().collect();
    let mut opts = Options::new("=:");
    opts.aliased_flag(&["-h", "--help"], "Display this information")
        .opt(
            "-tl",
            "Set time limit for executable (user process time)",
            "<number>[unit]",
        ).opt(
            "-d",
            "Set time limit for executable (wall-clock time)",
            "<number>[unit]",
        ).opt("-s", "Set security level to 0 or 1", "{0|1}")
        .opt("-ml", "Set memory limit for executable", "<number>[unit]")
        .opt("-wl", "Set write limit for executable", "<number>[unit]")
        .opt(
            "-y",
            "Set idleness time limit for executable",
            "<number>[unit]",
        ).opt(
            "-lr",
            "Required load of the processor for this executable not to be considered idle",
            "<number>[unit]",
        ).opt("-sw", "Display program window on the screen", "{0|1}")
        .opt("--debug", "", "{0|1}")
        .aliased_opt(
            &["-mi", "--monitorInterval"],
            "Sleep interval for a monitoring thread (default: 0.001s)",
            "<number>[unit]",
        ).opt("-wd", "Set working directory", "<dir>")
        .opt("-hr", "Do not display report on console", "{0|1}")
        .opt("-ho", "Do not display output on console", "{0|1}")
        .aliased_opt(
            &["-runas", "--delegated"],
            "Run spawner as delegate",
            "{0|1}",
        ).opt("-u", "Run executable under <user>", "<user>")
        .opt("-p", "Password for <user>", "<password>")
        .aliased_flag(
            &["-c", "--systempath"],
            "Search for executable in system path",
        ).opt("-sr", "Save report to <file>", "<file>")
        .opt(
            "-env",
            "Set environment variables for executable (default: inherit)",
            "{inherit|user-default|clear}",
        ).aliased_opt(
            &["-ff", "--file-flags"],
            "Set default flags for opened files (f - force flush, e - exclusively open)",
            "<flags>",
        ).opt(
            "-D",
            "Define additional environment variable for executable",
            "<var>",
        ).aliased_opt(
            &["-i", "--in"],
            "Redirect stdin from [*[<file-flags>]:]<filename>\n\
             or *[[<pipe-flags>]:]{null|std|<index>.stdout}",
            "<value>",
        ).aliased_opt(
            &["-so", "--out"],
            "Redirect stdout to [*[<file-flags>]:]<filename>\n\
             or *[[<pipe-flags>]:]{null|std|<index>.stdin}",
            "<value>",
        ).aliased_opt(
            &["-e", "-se", "--err"],
            "Redirect stderr to [*[<file-flags>]:]<filename>\n\
             or *[[<pipe-flags>]:]{null|std|<index>.stderr}",
            "<value>",
        ).opt("--separator", "Use <sep> to separate executables", "<sep>")
        .opt("-process-count", "", "<number>[unit]")
        .flag("--controller", "Mark executable as controller")
        .opt("--shared-memory", "", "<value>")
        .aliased_flag(&["-j", "--json"], "Use JSON format in report");

    let matches = opts.parse(&args);
    if args.len() < 2 || matches.has("-h") {
        println!("{}", opts.help("sp [options] executable [arguments]", ""));
        return;
    }

    if let Some(opt) = matches.unrecognized_opts().first() {
        match opt.chars().next() {
            Some(c) => match c {
                '-' => println!("Unknown argument {}", opt),
                _ => {}
            },
            None => {}
        }
    }
}
