use proc_macro2::{Literal, TokenStream};
use quote::{quote, ToTokens};
use syn::{
    Attribute, Data, DeriveInput, Error, Field, Lit, Meta, MetaList, MetaNameValue, NestedMeta,
};

struct OptKindOpt {
    value_desc: Option<String>,
    parser: Option<TokenStream>,
}

enum OptKind {
    Invalid,
    Opt(OptKindOpt),
    Flag,
}

struct Opt<'a> {
    kind: OptKind,
    names: Vec<String>,
    desc: Option<String>,
    env: Option<String>,
    field: &'a Field,
}

enum OptAttribute<'a> {
    Name(&'a MetaNameValue, String),
    Names(&'a MetaList, Vec<String>),
    Desc(&'a MetaNameValue, String),
    ValueDesc(&'a MetaNameValue, String),
    Parser(&'a MetaNameValue, String),
    Env(&'a MetaNameValue, String),
}

enum OptContainerAttribute {
    Overview(String),
    Delimeters(String),
    Usage(String),
    DefaultParser(String),
}

struct OptContainer<'a> {
    delimeters: Option<String>,
    usage: Option<String>,
    overview: Option<String>,
    default_parser: Option<TokenStream>,
    opts: Vec<Opt<'a>>,
    ast: &'a DeriveInput,
}

impl Default for OptKindOpt {
    fn default() -> Self {
        Self {
            value_desc: None,
            parser: None,
        }
    }
}

impl<'a> OptAttribute<'a> {
    fn names_from_meta_list(list: &'a MetaList) -> Result<Self, Error> {
        let mut names: Vec<String> = Vec::new();
        for item in list.nested.iter() {
            match item {
                NestedMeta::Literal(l) => names.push(expect_str(l)?),
                NestedMeta::Meta(m) => {
                    return Err(Error::new_spanned(m, "Expected string literal"));
                }
            }
        }
        Ok(OptAttribute::Names(list, names))
    }

    fn expected_one_of_err<T: ToTokens>(v: &T) -> Error {
        Error::new_spanned(
            v,
            "Expected one of: name = \"...\", names(...), desc = \"...\", \
             value_desc = \"...\" parser = \"...\" env = \"...\"",
        )
    }

    fn from_name_value(nameval: &'a MetaNameValue) -> Result<Self, Error> {
        let lit = &nameval.lit;
        match nameval.ident.to_string().as_str() {
            "name" => Ok(OptAttribute::Name(nameval, expect_str(lit)?)),
            "desc" => Ok(OptAttribute::Desc(nameval, expect_str(lit)?)),
            "value_desc" => Ok(OptAttribute::ValueDesc(nameval, expect_str(lit)?)),
            "parser" => Ok(OptAttribute::Parser(nameval, expect_str(lit)?)),
            "env" => Ok(OptAttribute::Env(nameval, expect_str(lit)?)),
            _ => Err(OptAttribute::expected_one_of_err(nameval)),
        }
    }

    fn from_meta(meta: &'a Meta) -> Result<Self, Error> {
        match meta {
            Meta::List(list) => {
                if list.ident != "names" {
                    Err(OptAttribute::expected_one_of_err(meta))
                } else {
                    OptAttribute::names_from_meta_list(&list)
                }
            }
            Meta::NameValue(nameval) => OptAttribute::from_name_value(&nameval),
            _ => Err(OptAttribute::expected_one_of_err(meta)),
        }
    }
}

impl<'a> Opt<'a> {
    fn new(kind: OptKind, field: &'a Field) -> Self {
        Opt {
            kind,
            names: Vec::new(),
            desc: None,
            env: None,
            field,
        }
    }

    fn from_meta_list(field: &'a Field, list: &MetaList) -> Result<Self, Error> {
        let mut attrs: Vec<OptAttribute> = Vec::new();
        for item in list.nested.iter() {
            match item {
                NestedMeta::Meta(m) => attrs.push(OptAttribute::from_meta(&m)?),
                _ => return Err(OptAttribute::expected_one_of_err(item)),
            }
        }

        let kind = match list.ident.to_string().as_str() {
            "opt" => OptKind::Opt(OptKindOpt::default()),
            "flag" => OptKind::Flag,
            _ => OptKind::Invalid,
        };

        let mut opt = Opt::new(kind, field);
        for attr in attrs.into_iter() {
            match attr {
                OptAttribute::Name(_, s) => opt.names = vec![s],
                OptAttribute::Names(_, v) => opt.names = v,
                OptAttribute::Desc(_, s) => opt.desc = Some(s),
                OptAttribute::ValueDesc(nameval, s) => match opt.kind {
                    OptKind::Opt(ref mut v) => v.value_desc = Some(s),
                    _ => {
                        return Err(Error::new_spanned(
                            nameval,
                            "Value description allowed on options only",
                        ));
                    }
                },
                OptAttribute::Parser(nameval, s) => match opt.kind {
                    OptKind::Opt(ref mut v) => v.parser = Some(s.parse().unwrap()),
                    _ => {
                        return Err(Error::new_spanned(
                            nameval,
                            "Parser allowed on options only",
                        ));
                    }
                },
                OptAttribute::Env(_, s) => opt.env = Some(s),
            }
        }

        if opt.names.is_empty() {
            return Err(Error::new_spanned(list, "Unnamed options are not allowed"));
        }

        Ok(opt)
    }

    fn from_meta(field: &'a Field, attr: &Attribute, meta: Option<Meta>) -> Result<Self, Error> {
        if let Some(m) = meta {
            if let Meta::List(list) = m {
                return Opt::from_meta_list(field, &list);
            }
        }
        Err(Error::new_spanned(
            attr,
            "Invalid attribute in #[opt(...)] or in #[flag(...)]",
        ))
    }

    fn from_field(field: &'a Field) -> Result<Vec<Self>, Error> {
        let mut opts: Vec<Self> = Vec::new();
        for attr in field.attrs.iter().rev() {
            if attr.path.segments.len() == 1 {
                let ident = &attr.path.segments[0].ident;
                if ident == "opt" || ident == "flag" {
                    opts.push(Opt::from_meta(field, attr, attr.interpret_meta())?);
                }
            }
        }
        if opts.is_empty() {
            opts.push(Opt::new(OptKind::Invalid, field));
        }
        Ok(opts)
    }
}

impl OptContainerAttribute {
    fn expected_one_of_err<T: ToTokens>(v: &T) -> Error {
        Error::new_spanned(
            v,
            "Expected one of: delimeters = \"...\", usage = \"...\", overview = \"...\" \
             default_parser = \"...\"",
        )
    }

    fn from_meta(meta: &Meta) -> Result<Self, Error> {
        if let Meta::NameValue(nameval) = meta {
            match nameval.ident.to_string().as_ref() {
                "overview" => Ok(OptContainerAttribute::Overview(expect_str(&nameval.lit)?)),
                "delimeters" => Ok(OptContainerAttribute::Delimeters(expect_str(&nameval.lit)?)),
                "usage" => Ok(OptContainerAttribute::Usage(expect_str(&nameval.lit)?)),
                "default_parser" => Ok(OptContainerAttribute::DefaultParser(expect_str(
                    &nameval.lit,
                )?)),
                _ => Err(OptContainerAttribute::expected_one_of_err(meta)),
            }
        } else {
            Err(OptContainerAttribute::expected_one_of_err(meta))
        }
    }
}

impl<'a> OptContainer<'a> {
    fn parse_meta_list(list: &MetaList) -> Result<Vec<OptContainerAttribute>, Error> {
        let mut attrs: Vec<OptContainerAttribute> = Vec::new();
        for item in list.nested.iter() {
            match item {
                NestedMeta::Meta(m) => attrs.push(OptContainerAttribute::from_meta(&m)?),
                _ => return Err(OptContainerAttribute::expected_one_of_err(item)),
            }
        }
        Ok(attrs)
    }

    fn parse_meta(
        attr: &Attribute,
        meta: Option<Meta>,
    ) -> Result<Vec<OptContainerAttribute>, Error> {
        if let Some(m) = meta {
            if let Meta::List(list) = m {
                return OptContainer::parse_meta_list(&list);
            }
        }
        Err(Error::new_spanned(
            attr,
            "Invalid attributes in #[optcont(...)]",
        ))
    }

    fn parse_attrs(attrs: &[Attribute]) -> Result<Vec<OptContainerAttribute>, Vec<Error>> {
        let mut errors: Vec<Error> = Vec::new();
        let mut result: Vec<OptContainerAttribute> = Vec::new();
        for attr in attrs.iter() {
            if attr.path.segments.len() == 1 && attr.path.segments[0].ident == "optcont" {
                match OptContainer::parse_meta(&attr, attr.interpret_meta()) {
                    Ok(att) => result.extend(att),
                    Err(e) => errors.push(e),
                }
            }
        }
        match errors.len() {
            0 => Ok(result),
            _ => Err(errors),
        }
    }

    fn init_opts(&mut self) -> Result<(), Vec<Error>> {
        let data = match self.ast.data {
            Data::Struct(ref data) => data,
            Data::Enum(_) => {
                return Err(vec![Error::new(
                    self.ast.ident.span(),
                    "Derive for enums is not supported",
                )]);
            }
            Data::Union(_) => {
                return Err(vec![Error::new(
                    self.ast.ident.span(),
                    "Derive for unions is not supported",
                )]);
            }
        };

        let mut errors: Vec<Error> = Vec::new();
        for field in data.fields.iter() {
            match Opt::from_field(field) {
                Ok(opts) => self.opts.extend(opts),
                Err(e) => errors.push(e),
            }
        }
        match errors.len() {
            0 => Ok(()),
            _ => Err(errors),
        }
    }

    fn init_attrs(&mut self) -> Result<(), Vec<Error>> {
        for att in OptContainer::parse_attrs(&self.ast.attrs)?.into_iter() {
            match att {
                OptContainerAttribute::Overview(s) => self.overview = Some(s),
                OptContainerAttribute::Delimeters(d) => self.delimeters = Some(d),
                OptContainerAttribute::Usage(u) => self.usage = Some(u),
                OptContainerAttribute::DefaultParser(p) => {
                    self.default_parser = Some(p.parse().unwrap())
                }
            }
        }
        Ok(())
    }

    fn from_ast(ast: &'a DeriveInput) -> Result<Self, Vec<Error>> {
        let mut cont = Self {
            delimeters: None,
            overview: None,
            usage: None,
            default_parser: None,
            opts: Vec::new(),
            ast,
        };
        cont.init_opts()?;
        cont.init_attrs()?;
        Ok(cont)
    }

    fn build_str_opt(&self, opt: &Option<String>) -> TokenStream {
        match opt {
            Some(s) => quote!(Some(#s.to_string())),
            None => quote!(None),
        }
    }

    fn build_help_fn(&self) -> TokenStream {
        let overview = self.build_str_opt(&self.overview);
        let usage = self.build_str_opt(&self.usage);
        let delimeters = self.build_str_opt(&self.delimeters);
        let options: Vec<TokenStream> = self
            .opts
            .iter()
            .filter_map(|opt| {
                let names: Vec<TokenStream> =
                    opt.names.iter().map(|s| quote!(#s.to_string())).collect();
                let desc = self.build_str_opt(&opt.desc);
                let env = self.build_str_opt(&opt.env);
                match opt.kind {
                    OptKind::Invalid => None,
                    OptKind::Flag => Some(quote! {
                        spawner_opts::OptionHelp {
                            names: vec![#(#names),*],
                            desc: #desc,
                            value_desc: None,
                            env: #env,
                        }
                    }),
                    OptKind::Opt(ref v) => {
                        let vd = self.build_str_opt(&v.value_desc);
                        Some(quote! {
                            spawner_opts::OptionHelp {
                                names: vec![#(#names),*],
                                desc: #desc,
                                value_desc: #vd,
                                env: #env,
                            }
                        })
                    }
                }
            })
            .collect();
        quote! {
            fn help() -> spawner_opts::Help {
                spawner_opts::Help {
                    overview: #overview,
                    usage: #usage,
                    delimeters: #delimeters,
                    options: vec![#(#options),*],
                }
            }
        }
    }

    fn build_register_opts(&self) -> Vec<TokenStream> {
        self.opts
            .iter()
            .filter_map(|opt| {
                let member_func = match &opt.kind {
                    OptKind::Flag => quote!(flag),
                    OptKind::Opt(_) => quote!(opt),
                    _ => return None,
                };
                let names: Vec<Lit> = opt
                    .names
                    .iter()
                    .map(|name| Lit::new(Literal::string(name)))
                    .collect();
                Some(quote! {
                    parser.#member_func(&[#(#names),*]);
                })
            })
            .collect()
    }

    fn opt_parser<'b>(&'b self, opt: &'b Opt) -> Result<&'b TokenStream, Error> {
        if let OptKind::Opt(ref v) = opt.kind {
            if let Some(parser) = v.parser.as_ref().or_else(|| self.default_parser.as_ref()) {
                return Ok(parser);
            }
        }
        Err(Error::new_spanned(
            opt.field,
            "Unable to find parser for this field",
        ))
    }

    fn env_flag_parser<'b>(&'b self, flag: &'b Opt) -> Result<&'b TokenStream, Error> {
        if let OptKind::Flag = flag.kind {
            if let Some(parser) = self.default_parser.as_ref() {
                return Ok(parser);
            }
        }
        Err(Error::new_spanned(
            flag.field,
            "Unable to find parser for this field",
        ))
    }

    fn build_set_opts(&self) -> Result<Vec<TokenStream>, Vec<Error>> {
        let mut set_opts: Vec<TokenStream> = Vec::new();
        let mut errors: Vec<Error> = Vec::new();

        for opt in &self.opts {
            let field = &opt.field.ident;
            let name = Lit::new(Literal::string(
                opt.names.iter().next().unwrap_or(&String::from("")),
            ));
            match opt.kind {
                OptKind::Flag => set_opts.push(quote! {
                    if parser.has_flag(#name) {
                        assert_flag_type_is_bool(&self.#field);
                        self.#field = true;
                    }
                }),
                OptKind::Opt(_) => match self.opt_parser(opt) {
                    Ok(parser) => set_opts.push(quote! {
                        if let Some(entries) = parser.get_opt(#name) {
                            for e in entries {
                                #parser::parse(&mut self.#field, e)?;
                            }
                        }
                    }),
                    Err(e) => errors.push(e),
                },
                _ => {}
            }
        }

        match errors.len() {
            0 => Ok(set_opts),
            _ => Err(errors),
        }
    }

    fn build_parse_env(&self) -> Result<Vec<TokenStream>, Vec<Error>> {
        let mut result = Vec::new();
        let mut errors = Vec::new();

        for opt in &self.opts {
            let env = match opt.env {
                Some(ref env) => env,
                _ => continue,
            };
            let parser = match opt.kind {
                OptKind::Flag => self.env_flag_parser(opt),
                OptKind::Opt(_) => self.opt_parser(opt),
                _ => continue,
            };

            let field = &opt.field.ident;
            match parser {
                Ok(parser) => result.push(quote! {
                    if let Some(val) = std::env::var(#env).ok() {
                        #parser::parse(&mut self.#field, val.as_str())?;
                    }
                }),
                Err(e) => errors.push(e),
            }
        }
        match errors.len() {
            0 => Ok(result),
            _ => Err(errors),
        }
    }

    fn build_parse_env_fn(&self) -> Result<TokenStream, Vec<Error>> {
        let parse_env = self.build_parse_env()?;
        Ok(quote! {
            fn parse_env(&mut self) -> std::result::Result<(), String> {
                #(#parse_env)*
                Ok(())
            }
        })
    }

    fn build_parse_argv_fn(&self) -> Result<TokenStream, Vec<Error>> {
        let delimeters = &self.delimeters;
        let register_opts = self.build_register_opts();
        let set_opts = self.build_set_opts()?;

        Ok(quote! {
            fn parse_argv<T, U>(&mut self, argv: T) -> std::result::Result<usize, String>
            where
                T: IntoIterator<Item = U>,
                U: AsRef<str>
            {
                use spawner_opts::parser::Parser;
                fn assert_flag_type_is_bool(v: &bool) {}

                let mut parser = Parser::new(argv, #delimeters);
                #(#register_opts)*
                let parsed_opts = parser.parse();
                #(#set_opts)*
                Ok(parsed_opts)
            }
        })
    }
}

fn expect_str(lit: &Lit) -> Result<String, Error> {
    match lit {
        Lit::Str(s) => Ok(s.value()),
        _ => Err(Error::new_spanned(lit, "Expected string literal")),
    }
}

pub fn expand_derive_cmd_line_options(ast: &DeriveInput) -> Result<TokenStream, Vec<Error>> {
    let cont = OptContainer::from_ast(ast)?;
    if let Data::Struct(_) = ast.data {
        let struct_name = &ast.ident;
        let help_fn = cont.build_help_fn();
        let parse_argv_fn = cont.build_parse_argv_fn()?;
        let parse_env_fn = cont.build_parse_env_fn()?;
        Ok(quote! {
            impl CmdLineOptions for #struct_name {
                #help_fn
                #parse_argv_fn
                #parse_env_fn
            }
        })
    } else {
        Err(Vec::new())
    }
}
