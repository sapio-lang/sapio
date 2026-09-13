use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use std::collections::{BTreeMap, BTreeSet};
use syn::{
    parse::{Parse, ParseStream},
    punctuated::Punctuated,
    spanned::Spanned,
    Attribute, Expr, FnArg, Ident, ImplItem, ImplItemMethod, ItemImpl, Path, Token,
};

struct OptionItem {
    name: Ident,
    value: OptionValue,
}
enum OptionValue {
    Flag,
    Expr(Box<Expr>),
    Paths(Vec<Path>),
}
struct Options(Vec<OptionItem>);
impl Parse for Options {
    fn parse(input: ParseStream) -> syn::Result<Self> {
        let mut options = vec![];
        let mut seen = BTreeSet::new();
        while !input.is_empty() {
            let name: Ident = input.parse()?;
            if !seen.insert(name.to_string()) {
                return Err(syn::Error::new(
                    name.span(),
                    format!("duplicate `{name}` option"),
                ));
            }
            let value = if input.peek(Token![=]) {
                input.parse::<Token![=]>()?;
                OptionValue::Expr(Box::new(input.parse()?))
            } else if input.peek(syn::token::Paren) {
                let content;
                syn::parenthesized!(content in input);
                OptionValue::Paths(
                    Punctuated::<Path, Token![,]>::parse_terminated(&content)?
                        .into_iter()
                        .collect(),
                )
            } else {
                OptionValue::Flag
            };
            options.push(OptionItem { name, value });
            if input.is_empty() {
                break;
            }
            input.parse::<Token![,]>()?;
        }
        Ok(Self(options))
    }
}
fn marker(attr: &Attribute) -> Option<&'static str> {
    [
        "action",
        "policy",
        "spend",
        "condition",
        "amount",
        "internal_key",
        "metadata",
    ]
    .into_iter()
    .find(|name| attr.path.is_ident(name))
}
fn paths(value: OptionValue, name: &Ident) -> syn::Result<Vec<Path>> {
    match value {
        OptionValue::Paths(paths) => Ok(paths),
        _ => Err(syn::Error::new(
            name.span(),
            "expected a parenthesized list of Rust paths",
        )),
    }
}
fn expr(value: OptionValue, name: &Ident) -> syn::Result<Expr> {
    match value {
        OptionValue::Expr(value) => Ok(*value),
        _ => Err(syn::Error::new(
            name.span(),
            "expected `name = RustExpression`",
        )),
    }
}
fn flag(value: OptionValue, name: &Ident) -> syn::Result<()> {
    match value {
        OptionValue::Flag => Ok(()),
        _ => Err(syn::Error::new(
            name.span(),
            "expected a flag without a value",
        )),
    }
}
fn opts(attr: &Attribute) -> syn::Result<Options> {
    if attr.tokens.is_empty() {
        Ok(Options(vec![]))
    } else {
        attr.parse_args()
    }
}
fn validate(method: &ImplItemMethod, counts: &[usize]) -> syn::Result<()> {
    let sig = &method.sig;
    if sig.asyncness.is_some()
        || sig.unsafety.is_some()
        || sig.abi.is_some()
        || sig.variadic.is_some()
        || !sig.generics.params.is_empty()
        || sig.generics.where_clause.is_some()
    {
        return Err(syn::Error::new_spanned(sig, "contract callbacks must be ordinary synchronous methods; put generic parameters on the impl"));
    }
    if !matches!(sig.inputs.first(), Some(FnArg::Receiver(r)) if r.reference.is_some() && r.mutability.is_none() && r.lifetime().is_none())
    {
        return Err(syn::Error::new_spanned(
            &sig.inputs,
            "contract callbacks require an ordinary `&self` receiver",
        ));
    }
    if !counts.contains(&sig.inputs.len()) {
        return Err(syn::Error::new_spanned(&sig.inputs, "unexpected callback arguments; actions take &self, Context and at most one typed request"));
    }
    Ok(())
}
fn local_name(path: &Path) -> Option<String> {
    if path.segments.len() == 2 && path.segments[0].ident == "Self" {
        Some(path.segments[1].ident.to_string())
    } else {
        None
    }
}
fn resolve(path: Path, known: &BTreeMap<String, Ident>) -> TokenStream {
    match local_name(&path).and_then(|name| known.get(&name)) {
        Some(factory) => quote!(Self::#factory),
        None => quote!(#path),
    }
}

pub(crate) fn expand(args: TokenStream, input: TokenStream) -> syn::Result<TokenStream> {
    let mut implementation: ItemImpl = syn::parse2(input)?;
    if implementation.trait_.is_some() {
        return Err(syn::Error::new_spanned(
            implementation.impl_token,
            "#[contract] belongs on an inherent impl",
        ));
    }
    let options: Options = syn::parse2(args)?;
    let mut exported_actions = vec![];
    let mut exported_guards = vec![];
    for OptionItem { name, value } in options.0 {
        match name.to_string().as_str() {
            "actions" => exported_actions.extend(paths(value, &name)?.into_iter().map(|p| quote!(#p))),
            "spends" => exported_guards.extend(paths(value, &name)?.into_iter().map(|p| quote!(#p))),
            _ => return Err(syn::Error::new(name.span(), "unknown contract option; use actions(...) or spends(...) for optional interface exports")),
        }
    }
    let mut policies = BTreeMap::new();
    let mut conditions = BTreeMap::new();
    let mut method_names = BTreeSet::new();
    for item in &implementation.items {
        if let ImplItem::Method(method) = item {
            method_names.insert(method.sig.ident.to_string());
            for attr in &method.attrs {
                match marker(attr) {
                    Some("policy" | "spend") => {
                        policies.insert(
                            method.sig.ident.to_string(),
                            format_ident!("__sapio_policy_{}", method.sig.ident),
                        );
                    }
                    Some("condition") => {
                        conditions.insert(
                            method.sig.ident.to_string(),
                            format_ident!("__sapio_condition_{}", method.sig.ident),
                        );
                    }
                    _ => {}
                }
            }
        }
    }
    let mut generated = vec![];
    let mut hooks = BTreeMap::new();
    for item in &mut implementation.items {
        let ImplItem::Method(method) = item else {
            continue;
        };
        let marks: Vec<_> = method
            .attrs
            .iter()
            .filter_map(|attr| marker(attr).map(|kind| (kind, attr.clone())))
            .collect();
        if marks.len() > 1 {
            return Err(syn::Error::new_spanned(
                method,
                "a method has exactly one contract role",
            ));
        }
        let Some((kind, attribute)) = marks.into_iter().next() else {
            continue;
        };
        method.attrs.retain(|attr| marker(attr).is_none());
        let cfg: Vec<_> = method
            .attrs
            .iter()
            .filter(|attr| attr.path.is_ident("cfg") || attr.path.is_ident("cfg_attr"))
            .collect();
        let name = &method.sig.ident;
        let vis = &method.vis;
        let name_text = name.to_string().trim_start_matches("r#").to_owned();
        match kind {
            "action" => {
                validate(method, &[2, 3])?;
                let mut committed = None;
                let mut guards = vec![];
                let mut compile_if = vec![];
                let mut defaults = None;
                let mut default = false;
                let mut local = false;
                let mut metadata = None;
                for OptionItem {
                    name: option,
                    value,
                } in opts(&attribute)?.0
                {
                    match option.to_string().as_str() {
                        "committed" | "suggested" => {
                            flag(value, &option)?;
                            if committed.replace(option == "committed").is_some() {
                                return Err(syn::Error::new(
                                    option.span(),
                                    "choose exactly one of committed or suggested",
                                ));
                            }
                        }
                        "guarded_by" => {
                            guards = paths(value, &option)?
                                .into_iter()
                                .map(|p| resolve(p, &policies))
                                .collect()
                        }
                        "compile_if" => {
                            compile_if = paths(value, &option)?
                                .into_iter()
                                .map(|p| resolve(p, &conditions))
                                .collect()
                        }
                        "defaults" => defaults = Some(expr(value, &option)?),
                        "default" => {
                            flag(value, &option)?;
                            default = true;
                        }
                        "local" => {
                            flag(value, &option)?;
                            local = true;
                        }
                        "simps" => metadata = Some(expr(value, &option)?),
                        _ => return Err(syn::Error::new(option.span(), "unknown action option")),
                    }
                }
                let committed = committed.ok_or_else(|| {
                    syn::Error::new(
                        attribute.span(),
                        "an action must explicitly be committed or suggested",
                    )
                })?;
                let has_request = method.sig.inputs.len() == 3;
                let request = if has_request {
                    let FnArg::Typed(arg) = &method.sig.inputs[2] else {
                        unreachable!()
                    };
                    let ty = &arg.ty;
                    quote!(#ty)
                } else {
                    quote!(())
                };
                if default && has_request {
                    return Err(syn::Error::new(attribute.span(), "`default` is for argument-free actions; use a separate `defaults` callback for typed requests"));
                }
                if default && defaults.is_some() {
                    return Err(syn::Error::new(
                        attribute.span(),
                        "choose default or defaults, not both",
                    ));
                }
                let kind = if committed {
                    quote!(Committed)
                } else {
                    quote!(Suggested)
                };
                let call = if has_request {
                    quote!(Self::#name(this, ctx, request))
                } else {
                    quote!(Self::#name(this, ctx))
                };
                let binding = if has_request {
                    quote!(request: #request)
                } else {
                    quote!((): ())
                };
                let callback = quote!(|this, ctx, #binding| ::sapio::contract::actions::IntoTemplates::into_templates(#call));
                let default_callback = if let Some(callback) = defaults {
                    quote!(.with_defaults(|this, ctx| ::sapio::contract::actions::IntoTemplates::into_templates((#callback)(this, ctx))))
                } else if default || (committed && !has_request) {
                    quote!(.with_defaults(|this, ctx| ::sapio::contract::actions::IntoTemplates::into_templates(Self::#name(this, ctx))))
                } else {
                    quote!()
                };
                let json = (!local).then(|| quote!(.with_json()));
                let metadata =
                    metadata.map(|f| quote!(.with_metadata(::std::option::Option::Some(#f))));
                let handle = format_ident!("{}_action", name);
                if method_names.contains(&handle.to_string()) {
                    return Err(syn::Error::new(
                        name.span(),
                        format!(
                            "generated action handle `{handle}` conflicts with an existing method"
                        ),
                    ));
                }
                generated.push(quote! {
                    #(#cfg)*
                    #[doc = "Typed action handle; direct invocation constructs proposals only."]
                    #vis fn #handle() -> ::sapio::contract::actions::Action<Self, #request> {
                        ::sapio::contract::actions::Action::new(#name_text, ::sapio::contract::actions::TemplateKind::#kind, #callback)
                            .with_guards(&[#(#guards),*])
                            .with_conditions(&[#(#compile_if),*])
                            #default_callback #metadata #json
                    }
                });
                exported_actions
                    .push(quote!(#(#cfg)* || ::std::option::Option::Some(Self::#handle().erase())));
            }
            "policy" | "spend" => {
                validate(method, &[1, 2])?;
                if !opts(&attribute)?.0.is_empty() {
                    return Err(syn::Error::new(attribute.span(), "policy methods declare their context dependency in their signature; no options are needed"));
                }
                let factory = &policies[&name.to_string()];
                if method_names.contains(&factory.to_string()) {
                    return Err(syn::Error::new(
                        name.span(),
                        "generated policy factory conflicts with a method",
                    ));
                }
                let (variant, callback) = if method.sig.inputs.len() == 1 {
                    (
                        quote!(CachedPolicy),
                        quote!(|this| ::sapio::sapio_base::policy::PolicyCompiler::compile_policy(&Self::#name(this)).map_err(::sapio::contract::CompilationError::from)),
                    )
                } else {
                    (
                        quote!(FreshPolicy),
                        quote!(|this, ctx| ::sapio::sapio_base::policy::PolicyCompiler::compile_policy(&Self::#name(this, ctx)).map_err(::sapio::contract::CompilationError::from)),
                    )
                };
                generated.push(quote! { #(#cfg)* fn #factory() -> ::std::option::Option<::sapio::contract::actions::Guard<Self>> {
                    ::std::option::Option::Some(::sapio::contract::actions::Guard::#variant(#callback, ::std::option::Option::None))
                }});
                if kind == "spend" {
                    exported_guards.push(quote!(#(#cfg)* Self::#factory));
                }
            }
            "condition" => {
                validate(method, &[2])?;
                if !opts(&attribute)?.0.is_empty() {
                    return Err(syn::Error::new(
                        attribute.span(),
                        "condition takes no options",
                    ));
                }
                let factory = &conditions[&name.to_string()];
                if method_names.contains(&factory.to_string()) {
                    return Err(syn::Error::new(
                        name.span(),
                        "generated condition factory conflicts with a method",
                    ));
                }
                generated.push(quote! { #(#cfg)* fn #factory() -> ::std::option::Option<::sapio::contract::actions::ConditionallyCompileIf<Self>> {
                    ::std::option::Option::Some(::sapio::contract::actions::ConditionallyCompileIf::Fresh(Self::#name))
                }});
            }
            hook => {
                validate(method, &[2])?;
                if !opts(&attribute)?.0.is_empty() {
                    return Err(syn::Error::new(
                        attribute.span(),
                        "contract hooks take no options",
                    ));
                }
                if hooks
                    .insert(
                        hook,
                        (name.clone(), cfg.into_iter().cloned().collect::<Vec<_>>()),
                    )
                    .is_some()
                {
                    return Err(syn::Error::new(name.span(), "duplicate contract hook"));
                }
            }
        }
    }
    let mut hook_impls = vec![];
    for (kind, (name, cfg)) in hooks {
        let delegation = match kind {
            "amount" => {
                quote!(fn ensure_amount(&self, ctx: ::sapio::Context) -> ::std::result::Result<::sapio::bitcoin::Amount, ::sapio::contract::CompilationError> { Self::#name(self, ctx) })
            }
            "internal_key" => {
                quote!(fn pinned_internal_key(&self, ctx: &::sapio::Context) -> ::std::result::Result<::std::option::Option<::sapio::bitcoin::XOnlyPublicKey>, ::sapio::contract::CompilationError> { Self::#name(self, ctx) })
            }
            "metadata" => {
                quote!(fn metadata(&self, ctx: ::sapio::Context) -> ::std::result::Result<::sapio::contract::object::ObjectMetadata, ::sapio::contract::CompilationError> { Self::#name(self, ctx) })
            }
            _ => unreachable!(),
        };
        hook_impls.push(quote!(#(#cfg)* #delegation));
    }
    let attrs = &implementation.attrs;
    let self_ty = &implementation.self_ty;
    let (impl_generics, ty_generics, where_clause) = implementation.generics.split_for_impl();
    // Inherent impl self types already contain their generic arguments.
    let _ = ty_generics;
    Ok(quote! {
        #implementation
        #(#attrs)*
        impl #impl_generics #self_ty #where_clause { #(#generated)* }
        #(#attrs)*
        impl #impl_generics ::sapio::contract::Contract for #self_ty #where_clause {
            const ACTIONS: &'static [::sapio::contract::actions::ActionFactory<Self>] = &[#(#exported_actions),*];
            const FINISH_FNS: &'static [fn() -> ::std::option::Option<::sapio::contract::actions::Guard<Self>>] = &[#(#exported_guards),*];
            #(#hook_impls)*
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn error(args: TokenStream, item: TokenStream) -> String {
        expand(args, item).unwrap_err().to_string()
    }
    #[test]
    fn actions_require_explicit_commitment_and_reject_unknown_or_duplicate_options() {
        assert!(error(
            quote!(),
            quote!(impl C { #[action] fn pay(&self, ctx: Context) -> Template {} })
        )
        .contains("explicitly be committed or suggested"));
        for flags in [quote!(committed, suggested), quote!(suggested, suggested)] {
            assert!(expand(
                quote!(),
                quote!(impl C { #[action(#flags)] fn pay(&self, ctx: Context) -> Template {} })
            )
            .is_err());
        }
        assert!(error(quote!(), quote!(impl C { #[action(suggested, guarded_byy(Self::key))] fn pay(&self, ctx: Context) -> Template {} })).contains("unknown action option"));
    }
    #[test]
    fn callbacks_keep_rust_receiver_semantics_and_reject_fake_default_requests() {
        assert!(error(
            quote!(),
            quote!(impl C { #[action(suggested)] fn pay(self, ctx: Context) -> Template {} })
        )
        .contains("ordinary `&self`"));
        assert!(error(quote!(), quote!(impl C { #[action(suggested, default)] fn pay(&self, ctx: Context, request: u64) -> Template {} })).contains("separate `defaults` callback"));
        assert!(error(
            quote!(),
            quote!(impl C { #[action(suggested)] async fn pay(&self, ctx: Context) -> Template {} })
        )
        .contains("ordinary synchronous methods"));
    }
    #[test]
    fn declared_roles_hooks_and_generated_names_are_unambiguous() {
        assert!(error(
            quote!(),
            quote!(impl C { #[spend] #[policy] fn signed(&self) -> Clause {} })
        )
        .contains("exactly one contract role"));
        assert!(error(quote!(), quote!(impl C { #[amount] fn one(&self, ctx: Context) -> Amount {} #[amount] fn two(&self, ctx: Context) -> Amount {} })).contains("duplicate contract hook"));
        assert!(error(quote!(), quote!(impl C { #[action(suggested)] fn pay(&self, ctx: Context) -> Template {} fn pay_action() {} })).contains("conflicts with an existing method"));
    }
    #[test]
    fn ordinary_methods_and_unquoted_policy_paths_survive_expansion() {
        let output = expand(quote!(), quote! {
            impl C {
                #[spend]
                pub fn signed(&self) -> Clause { policy_body() }
                #[action(suggested, guarded_by(Self::signed))]
                pub fn pay(&self, ctx: Context, amount: Amount) -> Result<Template, Error> { body(ctx, amount) }
            }
        }).unwrap();
        let file: syn::File = syn::parse2(output).unwrap();
        let syn::Item::Impl(original) = &file.items[0] else {
            panic!("ordinary inherent impl")
        };
        let ImplItem::Method(pay) = &original.items[1] else {
            panic!("ordinary method")
        };
        assert_eq!(pay.sig.ident, "pay");
        assert_eq!(pay.sig.inputs.len(), 3);
        assert_eq!(pay.block, syn::parse_quote!({ body(ctx, amount) }));
        assert!(file.items.len() == 3);
    }
}
