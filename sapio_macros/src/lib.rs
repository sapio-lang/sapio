// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! Attribute macros for Sapio contract actions.

use proc_macro::TokenStream;
use proc_macro2::TokenStream as Tokens;
use quote::{format_ident, quote};
use syn::{
    ext::IdentExt, parse_macro_input, parse_quote, AttributeArgs, FnArg, ItemFn, ReturnType,
};

mod parse;
use parse::{Action, Options};

/// Declare a conditional-compilation action with `(self, ctx: Context)`.
///
/// The body returns `ConditionalCompileType`. No options are accepted.
#[proc_macro_attribute]
pub fn compile_if(args: TokenStream, input: TokenStream) -> TokenStream {
    expand_attribute(Action::CompileIf, args, input)
}

/// Declare a guard with `(self, ctx: Context)` and a `Clause` body.
///
/// `#[guard(cached)]` instead accepts only `self`: a cached clause cannot depend
/// on its invocation context. `simps = "Some(Self::metadata)"` optionally supplies
/// a metadata callback, evaluated with the context of each guard attachment.
///
/// `#[guard(policy)]` accepts an explicit return type implementing
/// `sapio::sapio_base::policy::PolicyCompiler`. Its translation errors propagate
/// through contract compilation. Combine it with `cached` for context-free
/// policies evaluated once per compilation: `#[guard(policy, cached)]`.
#[proc_macro_attribute]
pub fn guard(args: TokenStream, input: TokenStream) -> TokenStream {
    expand_attribute(Action::Guard, args, input)
}

/// Declare a CTV action with `(self, ctx: Context)` and a `TxTmplIt` body.
///
/// Optional `guarded_by = "[Self::guard]"` and
/// `compile_if = "[Self::condition]"` arrays supply guard and condition factories.
/// Unknown or repeated options are errors, including misspelled guard names.
#[proc_macro_attribute]
pub fn then(args: TokenStream, input: TokenStream) -> TokenStream {
    expand_attribute(Action::Then, args, input)
}

/// Declare a continuation with `(self, ctx: Context, args: SpecificArgs)`.
///
/// `coerce_args = "Self::coerce"` is required. Optional `guarded_by` and
/// `compile_if` arrays work as on `then`; `web_api` enables JSON calls and
/// `simps = "Some(Self::metadata)"` supplies continuation metadata.
#[proc_macro_attribute]
pub fn continuation(args: TokenStream, input: TokenStream) -> TokenStream {
    expand_attribute(Action::Continuation, args, input)
}

fn expand_attribute(action: Action, args: TokenStream, input: TokenStream) -> TokenStream {
    let args = parse_macro_input!(args as AttributeArgs);
    let input = parse_macro_input!(input as ItemFn);
    expand(action, args, input)
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}

fn expand(action: Action, args: AttributeArgs, mut input: ItemFn) -> syn::Result<Tokens> {
    let options = Options::parse(action, args, input.sig.ident.span())?;
    parse::validate_signature(action, &options, &input.sig)?;
    let name = input.sig.ident.clone();
    let action_name = name.unraw().to_string();
    let attrs = input.attrs.clone();
    let vis = input.vis.clone();
    let helper = format_ident!("{}_{}", action.helper_prefix(), name);
    input.sig.ident = helper.clone();
    // The authoring syntax permits `self` as shorthand for the shared receiver
    // required by action callbacks. Other receiver forms are validated above.
    if let Some(FnArg::Receiver(receiver)) = input.sig.inputs.first_mut() {
        receiver.reference = Some((Default::default(), None));
    }
    if matches!(input.sig.output, ReturnType::Default) {
        input.sig.output = match action {
            Action::CompileIf => {
                parse_quote!(-> ::sapio::contract::actions::ConditionalCompileType)
            }
            Action::Guard => parse_quote!(-> ::sapio::sapio_base::Clause),
            Action::Then | Action::Continuation => parse_quote!(-> ::sapio::contract::TxTmplIt),
        };
    }
    input
        .attrs
        .push(parse_quote!(#[doc = "Implementation of the contract action."]));

    let Options {
        cached,
        policy,
        guarded_by,
        compile_if,
        coerce_args,
        simps,
        web_api,
    } = options;
    let factory = match action {
        Action::CompileIf => quote! {
            #(#attrs)*
            #[doc = "Conditional-compilation action declaration."]
            #vis fn #name() -> ::std::option::Option<::sapio::contract::actions::ConditionallyCompileIf<Self>> {
                ::std::option::Option::Some(
                    ::sapio::contract::actions::ConditionallyCompileIf::Fresh(Self::#helper)
                )
            }
        },
        Action::Guard => {
            let (variant, callback) = if policy {
                if cached {
                    (
                        quote!(CachedPolicy),
                        quote!(|this| {
                            ::sapio::sapio_base::policy::PolicyCompiler::compile_policy(&Self::#helper(this))
                                .map_err(::sapio::contract::CompilationError::from)
                        }),
                    )
                } else {
                    (
                        quote!(FreshPolicy),
                        quote!(|this, ctx| {
                            ::sapio::sapio_base::policy::PolicyCompiler::compile_policy(&Self::#helper(this, ctx))
                                .map_err(::sapio::contract::CompilationError::from)
                        }),
                    )
                }
            } else {
                (
                    if cached { quote!(Cache) } else { quote!(Fresh) },
                    quote!(Self::#helper),
                )
            };
            quote! {
                #(#attrs)*
                #[doc = "Guard action declaration."]
                #vis fn #name() -> ::std::option::Option<::sapio::contract::actions::Guard<Self>> {
                    ::std::option::Option::Some(
                        ::sapio::contract::actions::Guard::#variant(#callback, #simps)
                    )
                }
            }
        }
        Action::Then => {
            input
                .sig
                .inputs
                .push(parse_quote!(_: ::sapio::contract::actions::ThenFuncTypeTag));
            quote! {
                #(#attrs)*
                #[doc = "CTV action declaration."]
                #vis fn #name<'a>() -> ::std::option::Option<::sapio::contract::actions::ThenFuncAsFinishOrFunc<'a, Self, <Self as ::sapio::contract::Contract>::StatefulArguments>> {
                    ::std::option::Option::Some(::sapio::contract::actions::ThenFunc {
                        guard: &#guarded_by,
                        conditional_compile_if: &#compile_if,
                        func: Self::#helper,
                        name: ::std::sync::Arc::new(#action_name.into()),
                    }.into())
                }
            }
        }
        Action::Continuation => {
            let schema_helper = format_ident!("__sapio_schema_for_{}", name);
            let FnArg::Typed(argument) = &input.sig.inputs[2] else {
                unreachable!("validated continuation argument")
            };
            let ty = &argument.ty;
            let (web_api_type, schema) = if web_api {
                (
                    quote!(::sapio::contract::actions::WebAPIEnabled),
                    quote!(::std::option::Option::Some(::sapio::contract::macros::get_schema_for::<#ty>())),
                )
            } else {
                (
                    quote!(::sapio::contract::actions::WebAPIDisabled),
                    quote!(::std::option::Option::None),
                )
            };
            quote! {
                #(#attrs)*
                #[doc = "JSON schema for the continuation's specific arguments."]
                #vis fn #schema_helper() -> ::sapio::contract::macros::ContinuationSchema {
                    #schema
                }
                #(#attrs)*
                #[doc = "Continuation action declaration."]
                #vis fn #name<'a>() -> ::std::option::Option<::std::boxed::Box<dyn
                    ::sapio::contract::actions::CallableAsFoF<Self, <Self as ::sapio::contract::Contract>::StatefulArguments>>>
                {
                    let action: ::sapio::contract::actions::FinishOrFunc<_, _, _, #web_api_type> =
                        ::sapio::contract::actions::FinishOrFunc {
                            simp_gen: #simps,
                            coerce_args: #coerce_args,
                            guard: &#guarded_by,
                            conditional_compile_if: &#compile_if,
                            func: Self::#helper,
                            schema: Self::#schema_helper(),
                            name: ::std::sync::Arc::new(#action_name.into()),
                            f: ::std::default::Default::default(),
                            returned_txtmpls_modify_guards: false,
                            extract_clause_from_txtmpl: ::sapio::contract::actions::default_extract_clause_from_txtmpl,
                        };
                    ::std::option::Option::Some(::std::boxed::Box::new(action))
                }
            }
        }
    };
    Ok(quote!(#input #factory))
}

#[cfg(test)]
mod tests;
