pub struct Executable {
    path: String,
    args: Vec<String>,
}

impl Executable {
    pub fn new<T: Into<String>>(path: T) -> Executable {
        Executable {
            path: path.into(),
            args: Vec::new(),
        }
    }

    pub fn argument<T: Into<String>>(&mut self, arg: T) {
        self.args.push(arg.into())
    }

    pub fn stdin_from(&mut self) -> &mut Self {
        self
    }

    pub fn stdout_to(&mut self) -> &mut Self {
        self
    }

    pub fn stderr_to(&mut self) -> &mut Self {
        self
    }
}

impl<'a, T, U> From<T> for Executable
where
    T: IntoIterator<Item = U>,
    U: AsRef<str>,
{
    fn from(argv: T) -> Self {
        let mut argv = argv.into_iter();
        let mut e = Executable::new(argv.next().unwrap().as_ref().to_string());
        e.args.extend(argv.map(|x| x.as_ref().to_string()));
        e
    }
}
