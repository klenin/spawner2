use std::collections::HashMap;

pub struct Matches {
    values: Vec<String>,
    opts: HashMap<String, usize>,
    unrecognized_opts: Vec<String>,
}

impl Matches {
    fn new() -> Matches {
        Matches {
            values: Vec::new(),
            opts: HashMap::new(),
            unrecognized_opts: Vec::new(),
        }
    }

    pub fn unrecognized_opts(&self) -> &Vec<String> {
        &self.unrecognized_opts
    }

    pub fn get(&self, opt: &str) -> Option<String> {
        match self.opts.get(opt) {
            Some(v) => Some(self.values[*v].clone()),
            None => None,
        }
    }

    pub fn has(&self, opt: &str) -> bool {
        self.opts.get(opt).is_some()
    }
}

struct Opt {
    names: Vec<String>,
    desc: String,
    value_desc: String,
    has_value: bool,
}

pub struct Options {
    opts: Vec<Opt>,
    opt_name_delimeters: String,
}

impl Options {
    pub fn new(opt_name_delimeters: &str) -> Options {
        Options {
            opts: Vec::new(),
            opt_name_delimeters: opt_name_delimeters.to_string(),
        }
    }

    /// Creates an option that has multiple names and takes an argument
    pub fn aliased_opt(&mut self, names: &[&str], desc: &str, value_desc: &str) -> &mut Options {
        self.opts.push(Opt {
            names: names.iter().map(|x| x.to_string()).collect::<Vec<_>>(),
            desc: desc.to_string(),
            value_desc: value_desc.to_string(),
            has_value: true,
        });
        self
    }

    /// Creates an option that takes an argument
    pub fn opt(&mut self, name: &str, desc: &str, value_desc: &str) -> &mut Options {
        self.aliased_opt(&[name], desc, value_desc)
    }

    /// Creates an option that has multiple names and does not take an argument
    pub fn aliased_flag(&mut self, names: &[&str], desc: &str) -> &mut Options {
        self.opts.push(Opt {
            names: names.iter().map(|x| x.to_string()).collect::<Vec<_>>(),
            desc: desc.to_string(),
            value_desc: String::from(""),
            has_value: false,
        });
        self
    }

    /// Creates an option that does not take an argument
    pub fn flag(&mut self, name: &str, desc: &str) -> &mut Options {
        self.aliased_flag(&[name], desc)
    }

    fn parse_opt<'a>(&self, opt: &'a str) -> (&'a str, Option<&'a str>) {
        let mut name_len = 0;
        'outer: for c in opt.chars() {
            for d in self.opt_name_delimeters.chars() {
                if c == d {
                    break 'outer;
                }
            }
            name_len += 1;
        }

        let val = if name_len == opt.len() {
            None
        } else {
            Some(&opt[name_len..opt.len()])
        };

        (&opt[0..name_len], val)
    }

    pub fn parse(&self, args: &Vec<String>) -> Matches {
        let mut matches = Matches::new();

        if args.len() < 2 {
            return matches;
        }

        let mut optmap: HashMap<&str, &Opt> = HashMap::new();
        for opt in &self.opts {
            for name in &opt.names {
                optmap.insert(name.as_str(), opt);
            }
        }

        let mut i = 1;
        while i < args.len() {
            let (opt_name, opt_val) = self.parse_opt(args[i].as_str());
            let opt = match optmap.get(opt_name) {
                Some(v) => *v,
                None => break,
            };
            if !opt.has_value && opt_val.is_some() {
                break;
            }

            let mut val = "";
            if opt.has_value {
                val = if let Some(v) = opt_val {
                    v
                } else if i + 1 < args.len() {
                    i += 1;
                    args[i].as_str()
                } else {
                    break;
                };
            }

            matches.values.push(val.to_string());
            for name in &opt.names {
                matches.opts.insert(name.clone(), matches.values.len() - 1);
            }

            i += 1;
        }

        for j in i..args.len() {
            matches.unrecognized_opts.push(args[j].clone());
        }

        matches
    }

    fn opt_help(&self, opt: &Opt) -> String {
        let desc_offset = 30;
        let indent = String::from(" ").repeat(desc_offset);
        let delim = match self.opt_name_delimeters.chars().nth(0) {
            Some(v) => v,
            None => ' ',
        };

        let mut help = String::from("  ");
        for i in 0..opt.names.len() {
            if i > 0 {
                help.push_str(", ")
            }
            help.push_str(opt.names[i].as_str());
            if opt.has_value {
                help.push(delim);
                help.push_str(opt.value_desc.as_str());
            }
        }

        let mut is_first = true;
        for line in opt.desc.split("\n") {
            if line.is_empty() {
                continue;
            }
            let help_len = help.len();
            if is_first && help_len < desc_offset {
                help.push_str(String::from(" ").repeat(desc_offset - help_len).as_str());
            } else {
                help.push('\n');
                help.push_str(indent.as_str());
            }
            help.push_str(line);
            is_first = false;
        }

        help
    }

    pub fn help(&self, usage: &str, ending: &str) -> String {
        format!(
            "Usage: {}\nOptions:\n{}\n{}",
            usage,
            self.opts
                .iter()
                .map(|x| self.opt_help(x))
                .collect::<Vec<_>>()
                .join("\n"),
            ending
        )
    }
}
