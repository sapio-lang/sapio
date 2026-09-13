use proc_macro2::Span;
use std::collections::BTreeSet;
use syn::{
    parse_quote, spanned::Spanned, AttributeArgs, Expr, ExprArray, FnArg, Lit, LitStr, Meta,
    NestedMeta, ReturnType, Signature,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    CompileIf,
    Guard,
    Then,
    Continuation,
}

impl Action {
    pub(crate) fn helper_prefix(self) -> &'static str {
        match self {
            Self::CompileIf => "compile_if",
            Self::Guard => "guard",
            Self::Then => "then",
            Self::Continuation => "continue",
        }
    }

    fn accepts(self, name: &str) -> bool {
        match name {
            "cached" | "policy" => self == Self::Guard,
            "guarded_by" | "compile_if" => matches!(self, Self::Then | Self::Continuation),
            "defaults" | "default" | "web_api" => self == Self::Continuation,
            "simps" => matches!(self, Self::Guard | Self::Continuation),
            _ => false,
        }
    }
}

pub(crate) struct Options {
    pub cached: bool,
    pub policy: bool,
    pub guarded_by: ExprArray,
    pub compile_if: ExprArray,
    pub defaults: Option<Expr>,
    pub default: bool,
    pub simps: Expr,
    pub web_api: bool,
}

impl Options {
    pub(crate) fn parse(action: Action, args: AttributeArgs, span: Span) -> syn::Result<Self> {
        let mut options = Self {
            cached: false,
            policy: false,
            guarded_by: parse_quote!([]),
            compile_if: parse_quote!([]),
            defaults: None,
            default: false,
            simps: parse_quote!(::std::option::Option::None),
            web_api: false,
        };
        let mut seen = BTreeSet::new();
        for arg in args {
            let NestedMeta::Meta(meta) = arg else {
                return Err(syn::Error::new_spanned(
                    arg,
                    "expected a named action option",
                ));
            };
            let name = meta.path().get_ident().map(ToString::to_string);
            let Some(name) = name.filter(|name| action.accepts(name)) else {
                return Err(syn::Error::new_spanned(
                    meta.path(),
                    "unknown option for this action",
                ));
            };
            if !seen.insert(name.clone()) {
                return Err(syn::Error::new_spanned(
                    meta.path(),
                    format!("duplicate `{name}` option"),
                ));
            }
            match name.as_str() {
                "cached" | "policy" | "web_api" | "default" => {
                    if !matches!(meta, Meta::Path(_)) {
                        return Err(syn::Error::new_spanned(
                            meta,
                            format!("`{name}` is a flag; use `{name}` without a value"),
                        ));
                    }
                    if name == "cached" {
                        options.cached = true;
                    } else if name == "policy" {
                        options.policy = true;
                    } else if name == "default" {
                        options.default = true;
                    } else {
                        options.web_api = true;
                    }
                }
                "guarded_by" | "compile_if" => {
                    let literal = string_value(&meta)?;
                    let array = literal.parse::<ExprArray>().map_err(|_| {
                        syn::Error::new(literal.span(), format!("`{name}` must contain an array expression such as `[Self::action]`"))
                    })?;
                    if name == "guarded_by" {
                        options.guarded_by = array;
                    } else {
                        options.compile_if = array;
                    }
                }
                "defaults" => options.defaults = Some(string_value(&meta)?.parse()?),
                "simps" => options.simps = string_value(&meta)?.parse()?,
                _ => unreachable!("validated action option"),
            }
        }
        if options.default && options.defaults.is_some() {
            return Err(syn::Error::new(
                span,
                "choose `default` or `defaults`, not both",
            ));
        }
        Ok(options)
    }
}

fn string_value(meta: &Meta) -> syn::Result<&LitStr> {
    if let Meta::NameValue(value) = meta {
        if let Lit::Str(literal) = &value.lit {
            return Ok(literal);
        }
    }
    Err(syn::Error::new_spanned(
        meta,
        "expected `option = \"expression\"`",
    ))
}

pub(crate) fn validate_signature(
    action: Action,
    options: &Options,
    sig: &Signature,
) -> syn::Result<()> {
    if let Some(token) = &sig.asyncness {
        return Err(syn::Error::new(
            token.span(),
            "contract actions cannot be async",
        ));
    }
    if let Some(token) = &sig.constness {
        return Err(syn::Error::new(
            token.span(),
            "contract actions cannot be const",
        ));
    }
    if let Some(token) = &sig.unsafety {
        return Err(syn::Error::new(
            token.span(),
            "contract actions cannot be unsafe",
        ));
    }
    if let Some(abi) = &sig.abi {
        return Err(syn::Error::new_spanned(
            abi,
            "contract actions cannot specify an ABI",
        ));
    }
    if !sig.generics.params.is_empty() || sig.generics.where_clause.is_some() {
        return Err(syn::Error::new_spanned(&sig.generics, "contract actions cannot declare method generics or where clauses; put them on the impl"));
    }
    if let Some(variadic) = &sig.variadic {
        return Err(syn::Error::new_spanned(
            variadic,
            "contract actions cannot be variadic",
        ));
    }
    if options.policy && matches!(sig.output, ReturnType::Default) {
        return Err(syn::Error::new_spanned(
            &sig.ident,
            "a policy guard requires an explicit return type implementing PolicyCompiler",
        ));
    }
    let count = if options.cached {
        1
    } else if action == Action::Continuation {
        3
    } else {
        2
    };
    if sig.inputs.len() != count {
        let expected = if options.cached {
            "a cached guard takes only `self`; cached clauses cannot depend on Context"
        } else if action == Action::Continuation {
            "a continuation takes `self, ctx: Context, args: SpecificArgs`"
        } else {
            "this action takes `self, ctx: Context`"
        };
        return Err(syn::Error::new_spanned(&sig.inputs, expected));
    }
    match sig.inputs.first() {
        Some(FnArg::Receiver(receiver))
            if receiver.mutability.is_none() && receiver.lifetime().is_none() => {}
        _ => {
            return Err(syn::Error::new_spanned(
                &sig.inputs[0],
                "contract actions require `self` or `&self` as their first argument",
            ))
        }
    }
    for argument in sig.inputs.iter().skip(1) {
        if !matches!(argument, FnArg::Typed(_)) {
            return Err(syn::Error::new_spanned(
                argument,
                "expected a typed action argument",
            ));
        }
    }
    Ok(())
}
