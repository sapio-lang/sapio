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

mod contract;
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
/// No request is fabricated during compilation. Optional `guarded_by` and
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
        defaults,
        default,
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
        Action::Then => quote! {
            #(#attrs)*
            #[doc = "Committed transaction action declaration."]
            #vis fn #name() -> ::std::option::Option<::std::boxed::Box<dyn ::sapio::contract::actions::ErasedAction<Self>>> {
                ::std::option::Option::Some(
                    ::sapio::contract::actions::Action::new(
                        #action_name,
                        ::sapio::contract::actions::TemplateKind::Committed,
                        |this, ctx, ()| ::sapio::contract::actions::IntoTemplates::into_templates(Self::#helper(this, ctx)),
                    )
                    .with_guards(&#guarded_by)
                    .with_conditions(&#compile_if)
                    .with_defaults(|this, ctx| ::sapio::contract::actions::IntoTemplates::into_templates(Self::#helper(this, ctx)))
                    .erase()
                )
            }
        },
        Action::Continuation => {
            let schema_helper = format_ident!("__sapio_schema_for_{}", name);
            let FnArg::Typed(argument) = &input.sig.inputs[2] else {
                unreachable!("validated continuation argument")
            };
            let ty = &argument.ty;
            let schema = if web_api {
                quote!(::std::option::Option::Some(::sapio::contract::macros::get_schema_for::<#ty>()))
            } else {
                quote!(::std::option::Option::None)
            };
            let json = web_api.then(|| quote!(.with_json()));
            let default_proposals = if let Some(callback) = defaults {
                quote!(.with_defaults(|this, ctx| ::sapio::contract::actions::IntoTemplates::into_templates((#callback)(this, ctx))))
            } else if default {
                quote!(.with_defaults(|this, ctx| ::sapio::contract::actions::IntoTemplates::into_templates(Self::#helper(this, ctx, ()))))
            } else {
                quote!()
            };
            quote! {
                #(#attrs)*
                #[doc = "JSON schema for this action's request type."]
                #vis fn #schema_helper() -> ::sapio::contract::macros::ContinuationSchema { #schema }
                #(#attrs)*
                #[doc = "Suggested transaction action declaration."]
                #vis fn #name() -> ::std::option::Option<::std::boxed::Box<dyn ::sapio::contract::actions::ErasedAction<Self>>> {
                    ::std::option::Option::Some(
                        ::sapio::contract::actions::Action::new(
                            #action_name,
                            ::sapio::contract::actions::TemplateKind::Suggested,
                            |this, ctx, args: #ty| ::sapio::contract::actions::IntoTemplates::into_templates(Self::#helper(this, ctx, args)),
                        )
                        .with_guards(&#guarded_by)
                        .with_conditions(&#compile_if)
                        .with_metadata(#simps)
                        #default_proposals
                        #json
                        .erase()
                    )
                }
            }
        }
    };
    Ok(quote!(#input #factory))
}

#[cfg(test)]
mod tests;

/// Define a contract through ordinary Rust methods and explicitly marked actions.
/// See `sapio::contract::actions::Action` for request and default semantics.
#[proc_macro_attribute]
pub fn contract(args: TokenStream, input: TokenStream) -> TokenStream {
    contract::expand(args.into(), input.into())
        .unwrap_or_else(syn::Error::into_compile_error)
        .into()
}
