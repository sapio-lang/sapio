#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
}

use core::ops::Index;
use proc_macro::TokenStream;
use quote::{format_ident, quote};
use syn::Lit;
use syn::{parse_macro_input, AttributeArgs, ItemFn, Meta, NestedMeta};
/// The compile_if macro is used to define a `ConditionallyCompileIf`.
/// formats for calling are:
/// ```ignore
/// #[compile_if]
/// fn name(self, ctx: Context) {
///     /*ConditionallyCompileType*/
/// }
/// ```
#[proc_macro_attribute]
pub fn compile_if(args: TokenStream, input: TokenStream) -> TokenStream {
    let _args = parse_macro_input!(args as AttributeArgs);
    let input = parse_macro_input!(input as ItemFn);
    if input.sig.inputs.len() != 2 {
        panic!("Too may Arguments to function");
    }
    let context_arg = input.sig.inputs.index(1);
    let name = input.sig.ident;
    let compile_if_name = format_ident!("compile_if_{}", name);
    let block = input.block;
    proc_macro::TokenStream::from(quote! {
        fn #compile_if_name(&self, #context_arg) -> sapio::contract::actions::ConditionalCompileType
        #block
        fn #name() -> Option<sapio::contract::actions::ConditionallyCompileIf<Self>> {
            Some(sapio::contract::actions::ConditionallyCompileIf::Fresh(Self::#compile_if_name))
        }
    })
}

/// The guard macro is used to define a `Guard`. Guards may be cached or uncached.
/// formats for calling are:
/// ```ignore
/// #[guard(
///     /// optional, if desired to only be invoked once
///     cached
/// )]
/// fn name(self, ctx) {
///     /*Clause*/
/// }
/// ```
#[proc_macro_attribute]
pub fn guard(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as AttributeArgs);
    let input = parse_macro_input!(input as ItemFn);
    if input.sig.inputs.len() != 2 {
        panic!("Too may Arguments to function");
    }
    let context_arg = input.sig.inputs.index(1);
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
        fn #guard_name(&self, #context_arg) -> sapio::sapio_base::Clause
        #block
        fn  #name() -> Option<sapio::contract::actions::Guard<Self>> {
            Some(sapio::contract::actions::Guard::#ty(Self::#guard_name))
        }
    })
}

fn get_arrays(args: &Vec<NestedMeta>) -> (proc_macro2::TokenStream, proc_macro2::TokenStream) {
    let mut compile_if_array = None;
    let mut guarded_by_array = None;
    for arg in args {
        match (&compile_if_array, &guarded_by_array, arg) {
            (_, None, NestedMeta::Meta(Meta::NameValue(v))) if v.path.is_ident("guarded_by") => {
                match &v.lit {
                    Lit::Str(l) => {
                        guarded_by_array = Some(l.parse().expect("Token Stream Parsing"));
                    }
                    _ => panic!("Improperly Formatted {:?}", v),
                }
            }
            (_, Some(_), NestedMeta::Meta(Meta::NameValue(v))) if v.path.is_ident("guarded_by") => {
                panic!("Repeated guarded_by arguments");
            }
            (None, _, NestedMeta::Meta(Meta::NameValue(v))) if v.path.is_ident("compile_if") => {
                match &v.lit {
                    Lit::Str(l) => {
                        compile_if_array = Some(l.parse().expect("Token Stream Parsing"))
                    }
                    _ => panic!("Improperly Formatted {:?}", v),
                }
            }
            (Some(_), _, NestedMeta::Meta(Meta::NameValue(v))) if v.path.is_ident("compile_if") => {
                panic!("Repeated compile_if arguments");
            }
            _v => {}
        }
    }
    (
        compile_if_array.unwrap_or(quote! {[]}),
        guarded_by_array.unwrap_or(quote! {[]}),
    )
}

/// The then macro is used to define a `ThenFunction`.
/// formats for calling are:
/// ```ignore
/// /// A Conditional + Guarded CTV Function
/// #[then(
///     /// optional: only compile these branches if these compile_if statements permit
///     compile_if= "[compile_if_1, ... compile_if_n]",
///     /// optional: protect these branches with the conjunction (and) of these clauses
///     guarded_by= "[guard_1, ... guard_n]"
/// )]
/// fn name(self, ctx) {
///     /*Result<Box<Iterator<TransactionTemplate>>>*/
/// }
/// ```
#[proc_macro_attribute]
pub fn then(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as AttributeArgs);
    let input = parse_macro_input!(input as ItemFn);
    if input.sig.inputs.len() != 2 {
        panic!("Too may Arguments to function");
    }
    let context_arg = input.sig.inputs.index(1);
    let name = input.sig.ident;
    let then_fn_name = format_ident!("then_{}", name);
    let block = input.block;
    let (cia, gba) = get_arrays(&args);
    proc_macro::TokenStream::from(quote! {
            /// (missing docs fix)
            fn #name<'a>() -> Option<sapio::contract::actions::ThenFunc<'a, Self>>{
                Some(sapio::contract::actions::ThenFunc{
                    guard: &#gba,
                    conditional_compile_if: &#cia,
                    func: Self::#then_fn_name,
                    name: std::sync::Arc::new(std::stringify!(#name).into()),
                })
            }
            /// (missing docs fix)
            fn #then_fn_name(&self, #context_arg) -> sapio::contract::TxTmplIt
            #block
    })
}

fn web_api(args: &Vec<NestedMeta>) -> proc_macro2::TokenStream {
    for arg in args {
        match arg {
            NestedMeta::Meta(Meta::Path(v)) if v.is_ident("web_api") => {
                return quote! { sapio::contract::actions::WebAPIEnabled};
            }
            _ => continue,
        }
    }
    return quote! { sapio::contract::actions::WebAPIDisabled};
}
fn coerce_args(args: &Vec<NestedMeta>) -> proc_macro2::TokenStream {
    for arg in args {
        match arg {
            NestedMeta::Meta(Meta::NameValue(v)) if v.path.is_ident("coerce_args") => {
                match &v.lit {
                    Lit::Str(l) => {
                        return l.parse().expect("Token Stream Parsing");
                    }
                    _ => panic!("Improperly Formatted {:?}", v),
                }
            }
            _ => continue,
        }
    }
    panic!("No Coerce Arguments found");
}

fn web_api_schema(
    args: &Vec<NestedMeta>,
    name: &syn::Ident,
    typ: &syn::FnArg,
) -> proc_macro2::TokenStream {
    if let syn::FnArg::Typed(v) = typ {
        let ty = &v.ty;
        for arg in args {
            match arg {
                NestedMeta::Meta(Meta::Path(v)) if v.is_ident("web_api") => {
                    return quote! {
                    const #name : Option<&'static dyn Fn() -> std::sync::Arc<sapio::schemars::schema::RootSchema>> =
                        Some(&|| sapio::contract::macros::get_schema_for::<#ty>());
                    };
                }
                _ => continue,
            }
        }
    } else {
        panic!("Wrong type: {:?}", typ);
    }
    quote! {
        const #name : Option<&'static dyn Fn() -> std::sync::Arc<sapio::schemars::schema::RootSchema>> = None;
    }
}
///
///
/// The `continuation` macro generates a static `fn() -> Option<FinishOrFunc>` method for a given impl.
///
/// There are a few variants of how you can create a `continuation`.
///
/// ```ignore
/// struct UpdateType;
/// /// Helper
/// fn default_coerce(
///     k: <T as Contract>::StatefulArguments,
/// ) -> Result<UpdateType, CompilationError> {
///     Ok(k)
/// }
/// /// A Guarded CTV Function
/// #[continuation(
///     /// required: guards for the miniscript clauses required
///     guarded_by = "[Self::guard_1,... Self::guard_n]",
///     /// optional: Conditional compilation
///     compile_if = "[Self::compile_if_1, ... Self::compile_if_n]",
///     ///  optional: Enables compiling this for a json callable continuation
///     web_api,
///     /// helper for coercing args for json api, could be arbitrary
///     coerce_args = "default_coerce"
/// )]
/// fn name(self, ctx:Context, o:UpdateType) {
///     /*Result<Box<Iterator<TransactionTemplate>>>*/
/// }
/// /// Null Implementation
/// decl_finish!(name);
/// ```
#[proc_macro_attribute]
pub fn continuation(args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as AttributeArgs);
    let input = parse_macro_input!(input as ItemFn);
    let name = input.sig.ident;
    let continue_name = format_ident!("continue_{}", name);
    let block = input.block;
    if input.sig.inputs.len() != 3 {
        panic!("Too may Arguments to function");
    }
    let arg_type = input
        .sig
        .inputs
        .last()
        .expect("Must have at least one argument");
    let context_arg = input.sig.inputs.index(1);
    let (cia, gba) = get_arrays(&args);
    let web_api_type = web_api(&args);
    let continue_schema_for_name = format_ident!("continue_schema_for_{}", name);
    let web_api_schema_s = web_api_schema(&args, &continue_schema_for_name, &arg_type);
    let coerce_args_f = coerce_args(&args);
    proc_macro::TokenStream::from(quote! {
            #web_api_schema_s
            /// (missing docs fix)
            fn #continue_name(&self, #context_arg, #arg_type) -> sapio::contract::TxTmplIt
            #block
            /// (missing docs fix)
            fn #name<'a>() -> Option<Box<dyn
                sapio::contract::actions::CallableAsFoF<Self, <Self as sapio::contract::Contract>::StatefulArguments>>>
            {
                let f : sapio::contract::actions::FinishOrFunc<_, _, _, #web_api_type>= sapio::contract::actions::FinishOrFunc{
                    coerce_args: #coerce_args_f,
                    guard: &#gba,
                    conditional_compile_if: &#cia,
                    func: Self::#continue_name,
                    schema: Self::#continue_schema_for_name.map(|f|f()),
                    name: std::sync::Arc::new(std::stringify!(#name).into()),
                    f: std::default::Default::default()
                };
                Some(Box::new(f))
            }
    })
}
