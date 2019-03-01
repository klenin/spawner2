use std::collections::HashMap;

pub enum Entries {
    Flag(Vec<String>),
    Opt(Vec<String>),
}

pub struct Parser<T, U>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    pos: std::iter::Peekable<<T as IntoIterator>::IntoIter>,
    entries: Vec<Entries>,
    optmap: HashMap<&'static str, usize>,
    delims: &'static str,
}

impl<T, U> Parser<T, U>
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    pub fn new(argv: T, delims: &'static str) -> Self {
        Parser {
            pos: argv.into_iter().peekable(),
            entries: Vec::new(),
            optmap: HashMap::new(),
            delims: delims,
        }
    }

    fn add_names(&mut self, names: &[&'static str]) {
        let idx = self.entries.len() - 1;
        for name in names {
            self.optmap.insert(name, idx);
        }
    }

    pub fn opt(&mut self, names: &[&'static str]) -> &mut Self {
        self.entries.push(Entries::Opt(Vec::new()));
        self.add_names(names);
        self
    }

    pub fn flag(&mut self, names: &[&'static str]) -> &mut Self {
        self.entries.push(Entries::Flag(Vec::new()));
        self.add_names(names);
        self
    }

    pub fn has_flag(&self, flag: &str) -> bool {
        self.optmap.get(flag).map_or(false, |i| {
            if let Entries::Flag(ref e) = self.entries[*i] {
                e.len() != 0
            } else {
                false
            }
        })
    }

    pub fn get_opt(&self, opt: &str) -> Option<&Vec<String>> {
        self.optmap.get(opt).and_then(|i| {
            if let Entries::Opt(ref e) = self.entries[*i] {
                Some(e)
            } else {
                None
            }
        })
    }

    fn parse_opt(&mut self, arg: &str) -> bool {
        let (name, val) = match arg.find(|x| self.delims.find(x).is_some()) {
            Some(pos) => (&arg[0..pos], Some(&arg[pos + 1..arg.len()])),
            None => (&arg[0..arg.len()], None),
        };
        if let Some(opt_idx) = self.optmap.get(name) {
            let entries = &mut self.entries[*opt_idx];
            match (entries, val) {
                (Entries::Flag(e), None) => {
                    e.push(name.to_string());
                    true
                }
                (Entries::Opt(e), Some(v)) => {
                    e.push(v.to_string());
                    true
                }
                (Entries::Opt(e), None) => {
                    if let Some(next) = self.pos.next() {
                        e.push(next.as_ref().to_string());
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            }
        } else {
            false
        }
    }

    pub fn parse(&mut self) -> usize {
        let mut parsed_opts = 0;
        while let Some(arg) = self.pos.next() {
            if !self.parse_opt(arg.as_ref()) {
                break;
            }
            parsed_opts += 1;
        }
        parsed_opts
    }
}
