#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}

use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::{parse_macro_input, AttributeArgs, ItemFn, Meta, NestedMeta};
/// The compile_if macro is used to define a `ConditionallyCompileIf`.
/// formats for calling are:
/// ```ignore
/// compile_if!(fn name(self, ctx) {/*ConditionallyCompileType*/})
/// ```
#[proc_macro_attribute]
pub fn compile_if(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as AttributeArgs);
    let input = parse_macro_input!(input as ItemFn);
    let name = input.sig.ident;
    let compile_if_name = format_ident!("compile_if_{}", name);
    let block = input.block;
    proc_macro::TokenStream::from(quote! {
        fn #compile_if_name(&self, ctx: sapio::contract::Context) -> sapio::contract::actions::ConditionalCompileType
        #block
        fn #name() -> Option<sapio::contract::actions::ConditionallyCompileIf<Self>> {
            Some(sapio::contract::actions::ConditionallyCompileIf::Fresh(Self::#compile_if_name))
        }
    })
}


/// The guard macro is used to define a `Guard`. Guards may be cached or uncached.
/// formats for calling are:
/// ```ignore
/// guard!(fn name(self, ctx) {/*Clause*/})
/// /// The guard should only be invoked once
/// guard!(cached fn name(self, ctx) {/*Clause*/})
/// ```
#[proc_macro_attribute]
pub fn guard(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as AttributeArgs);
    let input = parse_macro_input!(input as ItemFn);
    let name = input.sig.ident;
    let guard_name = format_ident!("guard_{}", name);
    let block = input.block;
    let mut ty = format_ident!("Fresh");
    for arg in args {
        match arg {
            NestedMeta::Meta(Meta::NameValue(v)) if v.path.is_ident("cached") => {
                ty = format_ident!("Cached");
            }
            _ => {}
        }
    }
    proc_macro::TokenStream::from(quote! {
        fn #guard_name(&self, ctx: sapio::contract::Context) -> sapio::sapio_base::Clause
        #block
        fn  #name() -> Option<sapio::contract::actions::Guard<Self>> {
            Some(sapio::contract::actions::Guard::Fresh(Self::#guard_name))
        }
    })
}