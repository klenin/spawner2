//! This crate implements `CmdLineOptions` trait for a given struct, through Rust's `#derive`
//! mechanism. The crate should't be used directly, so in order to access its functionality
//! use `spawner_opts` library.
//!
//! # Container attributes
//! `#[optcont(delimeters = "...", usage = "...", default_parser = "...")]`
//! - `delimeters` - This tells parser on what character the incoming string should be split
//!   into the name\value pair.
//! - `usage` - This attribute helps to build proper help message.
//! - `default_parser` - If some field doesn't have the `parser` attribute the parser specified
//!   by `default_parser` will be used.
//!
//! # Field attributes
//! There are two kinds of field attributes:
//! - `#[opt(...)]`
//! - `#[flag(...)]`
//!
//! The main difference is that the fields marked by the `#[flag(...)]` macro must have `bool`
//! type, and the macro must not contain `value_desc` and `parser` attributes.
//!
//! # `#[flag(...)]` attributes
//! - `name = "--some_flag"` - The name of the flag.
//! - `names("-i", "--in")` - Multiple names of the same flag.
//! - `desc = "..."` - The description of the flag.
//!
//! # `#[opt(...)]` attributes
//! Shares the same attributes with the `#[flag(...)]` macro, including a few others:
//! - `parser = "IntValueParser"` - This attribute tells what parser should be used on the value.
//! The parser must implement `OptionValueParser` trait.
//! - `value_desc = "<int>"` - The description of the option's value.
#![recursion_limit = "128"]

extern crate proc_macro;
extern crate proc_macro2;
extern crate quote;
extern crate syn;

mod opts;

use opts::expand_derive_cmd_line_options;
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput, Error};

#[proc_macro_derive(CmdLineOptions, attributes(optcont, opt, flag))]
pub fn derive_cmd_line_options(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    expand_derive_cmd_line_options(&input)
        .unwrap_or_else(|errors| {
            let compile_errors = errors.iter().map(Error::to_compile_error);
            quote!(#(#compile_errors)*)
        })
        .into()
}
