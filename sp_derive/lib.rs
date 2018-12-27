#![allow(dead_code)]
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
