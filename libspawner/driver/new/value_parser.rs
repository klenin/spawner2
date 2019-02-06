use command::{EnvKind, EnvVar};
use driver::new::opts::{
    Options, PipeKind, RedirectFlags, StderrRedirectList, StdinRedirectList, StdioRedirect,
    StdioRedirectKind, StdioRedirectList, StdoutRedirectList,
};
use driver::prelude::OptionValueParser;
use std::time::Duration;

pub struct DefaultValueParser;
pub struct MemValueParser;
pub struct PercentValueParser;
pub struct StdinRedirectParser;
pub struct StdoutRedirectParser;
pub struct StderrRedirectParser;
pub struct FileFlagsParser;

impl OptionValueParser<Option<usize>> for DefaultValueParser {
    fn parse(opt: &mut Option<usize>, v: &str) -> Result<(), String> {
        if let Ok(v) = v.parse::<usize>() {
            *opt = Some(v);
            Ok(())
        } else {
            Err(format!("Invalid value '{}'", v))
        }
    }
}

impl OptionValueParser<bool> for DefaultValueParser {
    fn parse(opt: &mut bool, v: &str) -> Result<(), String> {
        if v.len() == 1 {
            match v.chars().next().unwrap() {
                '1' => {
                    *opt = true;
                    return Ok(());
                }
                '0' => {
                    *opt = false;
                    return Ok(());
                }
                _ => {}
            }
        }
        Err(format!(
            "Invalid value '{}'. Value must be either 0 or 1",
            v
        ))
    }
}

impl OptionValueParser<Option<String>> for DefaultValueParser {
    fn parse(opt: &mut Option<String>, v: &str) -> Result<(), String> {
        *opt = Some(v.to_string());
        Ok(())
    }
}

impl OptionValueParser<EnvKind> for DefaultValueParser {
    fn parse(env: &mut EnvKind, v: &str) -> Result<(), String> {
        match v {
            "clear" => *env = EnvKind::Clear,
            "inherit" => *env = EnvKind::Inherit,
            "user-default" => *env = EnvKind::UserDefault,
            _ => {
                return Err(format!(
                    "Unknown envieronment type '{}' expected one of: clear, inherit, user-default",
                    v
                ));
            }
        }
        Ok(())
    }
}

impl OptionValueParser<Vec<EnvVar>> for DefaultValueParser {
    fn parse(vars: &mut Vec<EnvVar>, v: &str) -> Result<(), String> {
        if let Some(pos) = v.find(|x| x == '=') {
            vars.push(EnvVar {
                name: v[0..pos].to_string(),
                val: v[pos + 1..v.len()].to_string(),
            });
            Ok(())
        } else {
            Err(format!(
                "Invalid envieronment variable '{}'. Expected NAME=VAL",
                v
            ))
        }
    }
}

impl OptionValueParser<Duration> for DefaultValueParser {
    fn parse(opt: &mut Duration, v: &str) -> Result<(), String> {
        *opt = parse_time_value(v)?;
        Ok(())
    }
}

impl OptionValueParser<Option<Duration>> for DefaultValueParser {
    fn parse(opt: &mut Option<Duration>, v: &str) -> Result<(), String> {
        *opt = Some(parse_time_value(v)?);
        Ok(())
    }
}

impl OptionValueParser<Option<f64>> for MemValueParser {
    fn parse(opt: &mut Option<f64>, v: &str) -> Result<(), String> {
        parse_value(v, parse_mem_degree, parse_mem_unit).map_or(
            Err(format!("Invalid value '{}'", v)),
            |(val, mult)| {
                *opt = Some(val * mult.map_or(1.0, |m| m / f64::powf(2.0, 20.0)));
                Ok(())
            },
        )
    }
}

impl OptionValueParser<f64> for PercentValueParser {
    fn parse(opt: &mut f64, v: &str) -> Result<(), String> {
        let (num_str, suffix) = split_number(v);
        if let Ok(num) = num_str.parse::<f64>() {
            if suffix == "" || suffix == "%" {
                *opt = num;
                return Ok(());
            }
        }
        Err(format!("Invalid value '{}'", v))
    }
}

macro_rules! check_redirect {
    ($redirect:expr, $expected:ident, invalid => ($a:ident, $b:ident)) => {{
        if let StdioRedirectKind::Pipe(ref kind) = $redirect.kind {
            match kind {
                PipeKind::$a(i) | PipeKind::$b(i) => Err(format!(
                    "Expected '{}' but got '{}' instead",
                    PipeKind::$expected(*i).to_string(),
                    kind.to_string()
                )),
                _ => Ok(()),
            }
        } else {
            Ok(())
        }
    }};
}

impl OptionValueParser<StdinRedirectList> for StdinRedirectParser {
    fn parse(opt: &mut StdinRedirectList, s: &str) -> Result<(), String> {
        if let Some(redirect) = parse_stdio_redirect(s, opt)? {
            check_redirect!(redirect, Stdout, invalid => (Stdin, Stderr))?;
            opt.items.push(redirect);
        }
        Ok(())
    }
}

impl OptionValueParser<StdoutRedirectList> for StdoutRedirectParser {
    fn parse(opt: &mut StdoutRedirectList, s: &str) -> Result<(), String> {
        if let Some(redirect) = parse_stdio_redirect(s, opt)? {
            check_redirect!(redirect, Stdin, invalid => (Stdout, Stderr))?;
            opt.items.push(redirect);
        }
        Ok(())
    }
}

impl OptionValueParser<StderrRedirectList> for StderrRedirectParser {
    fn parse(opt: &mut StderrRedirectList, s: &str) -> Result<(), String> {
        if let Some(redirect) = parse_stdio_redirect(s, opt)? {
            check_redirect!(redirect, Stderr, invalid => (Stdin, Stdout))?;
            opt.items.push(redirect);
        }
        Ok(())
    }
}

impl OptionValueParser<StdoutRedirectList> for FileFlagsParser {
    fn parse(opt: &mut StdoutRedirectList, s: &str) -> Result<(), String> {
        opt.default_flags = parse_redirect_flags(s, opt.default_flags)?;
        Ok(())
    }
}

fn split_number<'a>(num: &'a str) -> (&'a str, &'a str) {
    let len = num.len();
    let num_len = num.len()
        - num
            .chars()
            .rev()
            .position(|c| c.is_digit(10))
            .unwrap_or(len);
    (&num[0..num_len], &num[num_len..len])
}

fn parse_mem_unit(c: char) -> Option<f64> {
    match c {
        'b' => Some(1.0 / 8.0),
        'B' => Some(1.0),
        _ => None,
    }
}

fn parse_mem_degree(c: char) -> Option<f64> {
    match c {
        // C++ spawner sets value to 0 on these degrees.
        'd' => Some(0.0),
        'c' => Some(0.0),
        'm' => Some(0.0),
        'u' => Some(0.0),
        'n' => Some(0.0),
        'p' => Some(0.0),
        'f' => Some(0.0),
        '%' => Some(0.0),

        'k' => Some(f64::powf(2.0, 10.0)),
        'M' => Some(f64::powf(2.0, 20.0)),
        'G' => Some(f64::powf(2.0, 30.0)),
        'T' => Some(f64::powf(2.0, 40.0)),
        'P' => Some(f64::powf(2.0, 50.0)),
        _ => None,
    }
}

fn parse_time_unit(c: char) -> Option<f64> {
    match c {
        's' => Some(1.0),
        'm' => Some(60.0),
        'h' => Some(3600.0),
        'd' => Some(86400.0),
        _ => None,
    }
}

fn parse_time_degree(c: char) -> Option<f64> {
    match c {
        'd' => Some(1e-1),
        'c' => Some(1e-2),
        'm' => Some(1e-3),
        'u' => Some(1e-6),
        'n' => Some(1e-9),
        'p' => Some(1e-12),
        'f' => Some(1e-15),

        '%' => Some(0.01),
        'k' => Some(1e3),
        'M' => Some(1e6),
        'G' => Some(1e9),
        'T' => Some(1e12),
        'P' => Some(1e15),
        _ => None,
    }
}

fn parse_value<T, U>(s: &str, parse_degree: T, parse_unit: U) -> Option<(f64, Option<f64>)>
where
    T: Fn(char) -> Option<f64>,
    U: Fn(char) -> Option<f64>,
{
    let (num_str, suffix) = split_number(s);
    let mut suffix_chars = suffix.chars();
    num_str.parse::<f64>().ok().and_then(|v| {
        suffix_chars.next().map_or(Some((v, None)), |a| {
            suffix_chars.next().map_or(
                parse_degree(a)
                    .or(parse_unit(a))
                    .and_then(|mult| Some((v, Some(mult)))),
                |b| {
                    parse_degree(a).and_then(|degree| {
                        parse_unit(b).map_or(Some((v, Some(degree))), |unit| {
                            Some((v, Some(degree * unit)))
                        })
                    })
                },
            )
        })
    })
}

fn parse_time_value(v: &str) -> Result<Duration, String> {
    parse_value(v, parse_time_degree, parse_time_unit).map_or(
        Err(format!("Invalid value '{}'", v)),
        |(val, mult)| {
            let usec = (val * mult.unwrap_or(1.0) * 1e6) as u64;
            Ok(Duration::from_micros(usec))
        },
    )
}

fn parse_redirect_flags(
    s: &str,
    mut default_flags: RedirectFlags,
) -> Result<RedirectFlags, String> {
    let mut chars = s.chars();
    let mut value = true;
    while let Some(c) = chars.next() {
        match c {
            '-' => {
                value = false;
                continue;
            }
            'f' => {
                default_flags.flush = value;
                value = true;
            }
            'e' => {
                default_flags.exclusive = value;
                value = true;
            }
            _ => return Err(format!("Invalid flag '{}' in '{}'", c, s)),
        }
    }
    Ok(default_flags)
}

fn parse_pipe_redirect(s: &str, flags: RedirectFlags) -> Result<StdioRedirect, String> {
    if let Some(pos) = s.find(|c| c == '.') {
        let (num_str, pipe_kind) = (&s[0..pos], &s[pos + 1..s.len()]);
        usize::from_str_radix(num_str, 10).ok().map_or(
            Err(format!("Invalid pipe index '{}'", num_str)),
            |v| match pipe_kind {
                "stdin" => Ok(StdioRedirect::pipe(PipeKind::Stdin(v), flags)),
                "stdout" => Ok(StdioRedirect::pipe(PipeKind::Stdout(v), flags)),
                "stderr" => Ok(StdioRedirect::pipe(PipeKind::Stderr(v), flags)),
                _ => Err(format!("Invalid suffix '{}' in '{}'", pipe_kind, s)),
            },
        )
    } else {
        match s {
            "std" => Ok(StdioRedirect::pipe(PipeKind::Std, flags)),
            "null" => Ok(StdioRedirect::pipe(PipeKind::Null, flags)),
            _ => Err(format!("Invalid pipe redirect '{}'", s)),
        }
    }
}

fn parse_file_redirect(s: &str, flags: RedirectFlags) -> StdioRedirect {
    StdioRedirect::file(s.to_string(), flags)
}

fn parse_pipe_or_file_redirect(s: &str, flags: RedirectFlags) -> StdioRedirect {
    parse_pipe_redirect(s, flags)
        .ok()
        .unwrap_or(parse_file_redirect(s, flags))
}

fn parse_stdio_redirect(
    s: &str,
    list: &mut StdioRedirectList,
) -> Result<Option<StdioRedirect>, String> {
    let len = s.len();
    if !s.starts_with("*") {
        // file
        Ok(Some(parse_file_redirect(s, Options::DEFAULT_FILE_FLAGS)))
    } else if s.starts_with("*:") {
        // *: or *:file or *:n.stdio
        if len == 2 {
            list.default_flags = Options::DEFAULT_FILE_FLAGS;
            Ok(None)
        } else {
            Ok(Some(parse_pipe_or_file_redirect(
                &s[2..],
                list.default_flags,
            )))
        }
    } else if let Some(pos) = s.find(':') {
        // *flags:file or *flags:n.stdio or *flags:
        let flags = parse_redirect_flags(&s[1..pos], list.default_flags)?;
        let redirect = &s[pos + 1..];
        if redirect.len() == 0 {
            list.default_flags = flags;
            Ok(None)
        } else {
            Ok(Some(parse_pipe_or_file_redirect(redirect, flags)))
        }
    } else {
        // *n.stdio
        Ok(Some(parse_pipe_redirect(
            &s[1..],
            Options::DEFAULT_PIPE_FLAGS,
        )?))
    }
}
