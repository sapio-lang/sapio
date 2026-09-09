use super::*;
use syn::{
    parse::Parser, punctuated::Punctuated, ImplItem, ItemImpl, NestedMeta, Token, Visibility,
};

fn arguments(tokens: Tokens) -> AttributeArgs {
    Punctuated::<NestedMeta, Token![,]>::parse_terminated
        .parse2(tokens)
        .unwrap()
        .into_iter()
        .collect()
}

fn options(action: Action, tokens: Tokens) -> syn::Result<Options> {
    Options::parse(action, arguments(tokens), proc_macro2::Span::call_site())
}

fn option_error(action: Action, tokens: Tokens) -> String {
    match options(action, tokens) {
        Ok(_) => panic!("invalid options were accepted"),
        Err(error) => error.to_string(),
    }
}

#[test]
fn unknown_options_fail_instead_of_removing_contract_conditions() {
    for action in [
        Action::CompileIf,
        Action::Guard,
        Action::Then,
        Action::Continuation,
    ] {
        for tokens in [
            quote!(guarded_byy = "[Self::signed]"),
            quote!(compile_iff = "[Self::allowed]"),
            quote!(unknown),
        ] {
            assert_eq!(
                option_error(action, tokens),
                "unknown option for this action"
            );
        }
    }
    for (action, tokens) in [
        (Action::CompileIf, quote!(cached)),
        (Action::Guard, quote!(guarded_by = "[]")),
        (Action::Then, quote!(web_api)),
        (Action::Then, quote!(simps = "None")),
        (Action::Continuation, quote!(cached)),
    ] {
        assert_eq!(
            option_error(action, tokens),
            "unknown option for this action"
        );
    }
}

#[test]
fn duplicate_options_are_always_errors() {
    for (action, tokens, name) in [
        (Action::Guard, quote!(cached, cached), "cached"),
        (
            Action::Guard,
            quote!(simps = "None", simps = "None"),
            "simps",
        ),
        (
            Action::Then,
            quote!(guarded_by = "[]", guarded_by = "[]"),
            "guarded_by",
        ),
        (
            Action::Then,
            quote!(compile_if = "[]", compile_if = "[]"),
            "compile_if",
        ),
        (Action::Continuation, quote!(web_api, web_api), "web_api"),
        (
            Action::Continuation,
            quote!(coerce_args = "one", coerce_args = "two"),
            "coerce_args",
        ),
    ] {
        assert_eq!(
            option_error(action, tokens),
            format!("duplicate `{name}` option")
        );
    }
}

#[test]
fn options_require_their_documented_syntax() {
    for tokens in [
        quote!(cached = true),
        quote!(cached = false),
        quote!(cached()),
    ] {
        assert!(option_error(Action::Guard, tokens).contains("`cached` is a flag"));
    }
    for tokens in [
        quote!(guarded_by = 1),
        quote!(guarded_by),
        quote!(guarded_by(Self::signed)),
    ] {
        assert!(option_error(Action::Then, tokens).contains("expected `option ="));
    }
    for tokens in [
        quote!(guarded_by = "Self::signed"),
        quote!(guarded_by = "[Self::signed; 2]"),
        quote!(guarded_by = "[Self::signed] junk"),
    ] {
        assert!(option_error(Action::Then, tokens).contains("array expression"));
    }
    assert!(option_error(Action::Continuation, quote!()).contains("requires `coerce_args"));
    assert!(options(Action::Continuation, quote!(coerce_args = "{")).is_err());
    assert!(options(Action::Guard, quote!(simps = "Some(")).is_err());
    let valid = options(
        Action::Continuation,
        quote!(
            guarded_by = "[Self::signed, Self::timeout]",
            compile_if = "[Self::enabled]",
            coerce_args = "default_coerce::<Self>",
            simps = "Some(Self::metadata)",
            web_api,
        ),
    )
    .unwrap();
    assert_eq!(valid.guarded_by.elems.len(), 2);
    assert_eq!(valid.compile_if.elems.len(), 1);
    assert!(valid.web_api);
}

#[test]
fn unsupported_signatures_are_rejected_before_expansion() {
    for (source, message) in [
        ("async fn signed(self, ctx: Context) {}", "cannot be async"),
        ("const fn signed(self, ctx: Context) {}", "cannot be const"),
        (
            "unsafe fn signed(self, ctx: Context) {}",
            "cannot be unsafe",
        ),
        (
            "extern \"C\" fn signed(self, ctx: Context) {}",
            "specify an ABI",
        ),
        ("fn signed<T>(self, ctx: Context) {}", "method generics"),
        (
            "fn signed(self, ctx: Context) where Self: Clone {}",
            "where clauses",
        ),
        ("fn signed(ctx: Context, unused: ()) {}", "first argument"),
        (
            "fn signed(self: Box<Self>, ctx: Context) {}",
            "first argument",
        ),
        ("fn signed(&mut self, ctx: Context) {}", "first argument"),
        ("fn signed(mut self, ctx: Context) {}", "first argument"),
        (
            "fn signed(&'static self, ctx: Context) {}",
            "first argument",
        ),
        ("fn signed() {}", "takes `self, ctx"),
        (
            "fn signed(self, ctx: Context, extra: ()) {}",
            "takes `self, ctx",
        ),
    ] {
        let error = expand(Action::Guard, vec![], syn::parse_str(source).unwrap()).unwrap_err();
        assert!(error.to_string().contains(message), "{source}: {error}");
    }
    let error = expand(
        Action::Guard,
        arguments(quote!(cached)),
        parse_quote!(
            fn signed(self, ctx: Context) {}
        ),
    )
    .unwrap_err();
    assert!(error
        .to_string()
        .contains("cached clauses cannot depend on Context"));
    for receiver in [quote!(self), quote!(&self)] {
        let input = syn::parse2(quote!(fn signed(#receiver) {})).unwrap();
        assert!(expand(Action::Guard, arguments(quote!(cached)), input).is_ok());
    }
}

#[test]
fn expansion_preserves_attributes_visibility_patterns_and_return_types() {
    let expansion = expand(
        Action::Then,
        vec![],
        parse_quote! {
            #[doc = "A documented action."]
            #[cfg(feature = "payments")]
            #[allow(unused_variables)]
            pub(crate) fn pay(&self, mut context: Context) -> CustomResult { body(context) }
        },
    )
    .unwrap();
    let generated: ItemImpl = syn::parse2(quote!(impl Contract { #expansion })).unwrap();
    assert_eq!(generated.items.len(), 2);
    for item in generated.items {
        let ImplItem::Method(method) = item else {
            panic!("expected a method")
        };
        assert!(matches!(method.vis, Visibility::Restricted(_)));
        for attr in ["doc", "cfg", "allow"] {
            assert!(method.attrs.iter().any(|a| a.path.is_ident(attr)));
        }
        if method.sig.ident == "then_pay" {
            assert_eq!(method.sig.output, parse_quote!(-> CustomResult));
            assert_eq!(method.sig.inputs[1], parse_quote!(mut context: Context));
            assert_eq!(method.block, parse_quote!({ body(context) }));
        }
    }
}

#[test]
fn continuation_helpers_preserve_case_and_raw_identifiers() {
    let mut names = std::collections::BTreeSet::new();
    for ident in [quote!(pay), quote!(PAY), quote!(r#type)] {
        let expansion = expand(
            Action::Continuation,
            arguments(quote!(coerce_args = "Ok", web_api)),
            syn::parse2(quote! {
                #[allow(non_snake_case)]
                fn #ident(self, ctx: Context, args: ()) {}
            })
            .unwrap(),
        )
        .unwrap();
        let generated: ItemImpl = syn::parse2(quote!(impl Contract { #expansion })).unwrap();
        for item in generated.items {
            let ImplItem::Method(method) = item else {
                panic!("expected a method")
            };
            assert!(names.insert(method.sig.ident.to_string()));
            assert!(method.attrs.iter().any(|a| a.path.is_ident("allow")));
        }
    }
    assert!(names.contains("__sapio_schema_for_pay"));
    assert!(names.contains("__sapio_schema_for_PAY"));
    assert!(names.contains("__sapio_schema_for_type"));
}
