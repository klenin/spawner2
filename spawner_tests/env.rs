use crate::common::{read_all, TmpDir, APP};

use spawner_driver::run;

struct Env {
    data: String,
}

impl Env {
    fn new<T, U>(argv: T) -> Env
    where
        T: IntoIterator<Item = U>,
        U: AsRef<str>,
    {
        let tmp = TmpDir::new();
        let out = tmp.file("out.txt");
        let mut args = vec![format!("--out={}", out)];
        args.extend(argv.into_iter().map(|s| s.as_ref().to_string()));
        args.push(APP.to_string());
        args.push("print_env".to_string());
        run(args).unwrap();
        Env {
            data: read_all(out),
        }
    }

    fn vars<'a>(&'a self) -> Vec<(&'a str, &'a str)> {
        self.data
            .lines()
            .map(|line| {
                let eq_pos = line.find('=').unwrap();
                (&line[..eq_pos], &line[eq_pos + 1..])
            })
            .collect()
    }
}

#[test]
fn clear_env() {
    let env = Env::new(&["-env=clear"]);
    assert_eq!(env.vars(), Vec::new());
}

#[test]
fn define_var() {
    let env = Env::new(&["-env=clear", "-D:NAME=VAR"]);
    assert_eq!(env.vars(), vec![("NAME", "VAR")]);
}

#[test]
fn define_var_2() {
    let env = Env::new(&["-env=clear", "-D:A=B", "-D:C=D"]);
    let mut vars = env.vars();
    vars.sort_by(|a, b| a.0.partial_cmp(b.0).unwrap());
    assert_eq!(vars, vec![("A", "B"), ("C", "D")]);
}

#[test]
fn overwrite_var() {
    let env = Env::new(&["-env=clear", "-D:NAME=VAR", "-D:NAME=VAR1"]);
    assert_eq!(env.vars(), vec![("NAME", "VAR1")]);
}

#[test]
fn default_env() {
    let cmds: [&'static str; 0] = [];
    let env = Env::new(&cmds);
    assert!(env.vars().len() != 0);
}
