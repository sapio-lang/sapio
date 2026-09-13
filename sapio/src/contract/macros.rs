// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! macros for making defining Sapio contracts less verbose.
//!
//! Action options are checked at the declaration. A typo cannot silently omit
//! an authorization guard:
//! ```compile_fail
//! use sapio::{Context, contract::Contract};
//! struct Payment;
//! impl Payment {
//!     #[sapio::then(guarded_byy = "[Self::signed]")]
//!     fn pay(self, _ctx: Context) { sapio::contract::empty() }
//! }
//! impl Contract for Payment {}
//! ```
//! Cached clauses have no invocation context:
//! ```compile_fail
//! struct Payment;
//! impl Payment {
//!     #[sapio::guard(cached)]
//!     fn signed(self, _ctx: sapio::Context) { sapio::sapio_base::Clause::Trivial }
//! }
//! ```
//! Explicit return types are checked rather than discarded:
//! ```compile_fail
//! struct Payment;
//! impl Payment {
//!     #[sapio::guard]
//!     fn signed(self, _ctx: sapio::Context) -> bool { sapio::sapio_base::Clause::Trivial }
//! }
//! ```

use core::any::TypeId;
pub use paste::paste;

use serde_json::Value;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

/// Explicitly export action or independent spending-policy factories.
#[macro_export]
macro_rules! declare {
    {actions $(,$action:expr)* $(,)?} => {
        const ACTIONS: &'static [$crate::contract::actions::ActionFactory<Self>] = &[$($action,)*];
    };
    {finish $(,$guard:expr)* $(,)?} => {
        const FINISH_FNS: &'static [fn() -> ::std::option::Option<$crate::contract::actions::Guard<Self>>] = &[$($guard,)*];
    };
}

/// Declare an optional committed action in a contract interface.
#[macro_export]
macro_rules! decl_then {
    {$(#[$meta:meta])* $name:ident} => {
        $crate::contract::macros::paste! {
            $(#[$meta])*
            fn [<then_ $name>](&self, _ctx: $crate::contract::Context) -> $crate::contract::TxTmplIt {
                unimplemented!()
            }
            $(#[$meta])*
            fn $name() -> ::std::option::Option<::std::boxed::Box<dyn $crate::contract::actions::ErasedAction<Self>>> { ::std::option::Option::None }
        }
    };
}

lazy_static::lazy_static! {
static ref SCHEMA_MAP: Mutex<BTreeMap<TypeId, Arc<Value>>> =
Mutex::new(BTreeMap::new());
}
/// `get_schema_for` returns a cached schema for a given type.  this is
/// useful because we might expect to generate the same schema many times,
/// and they can use a decent amount of memory.
pub fn get_schema_for<T: schemars::JsonSchema + 'static + Sized>() -> Arc<Value> {
    SCHEMA_MAP
        .lock()
        .unwrap()
        .entry(TypeId::of::<T>())
        .or_insert_with(|| {
            Arc::new(
                serde_json::to_value(
                    schemars::generate::SchemaSettings::draft07()
                        .for_deserialize()
                        .into_generator()
                        .into_root_schema_for::<T>(),
                )
                .expect("Schema must be able to convert to JSON"),
            )
        })
        .clone()
}

/// The optional JSON schema of a continuation's specific argument type.
pub type ContinuationSchema = Option<Arc<Value>>;

/// Internal schema declaration helper for `decl_continuation!`.
#[macro_export]
macro_rules! web_api {
    {$(#[$meta:meta])* $name:ident,$type:ty,{}} => {
        $crate::contract::macros::paste!{
            $(#[$meta])*
            fn [<__sapio_schema_for_ $name>]() -> $crate::contract::macros::ContinuationSchema {
                ::std::option::Option::Some($crate::contract::macros::get_schema_for::<$type>())
            }
        }
    };
    {$(#[$meta:meta])* $name:ident,$type:ty} => {
        $crate::contract::macros::paste!{
            $(#[$meta])*
            fn [<__sapio_schema_for_ $name>]() -> $crate::contract::macros::ContinuationSchema {
                ::std::option::Option::None
            }
        }
    }
}
pub use web_api;

/// Declare an optional request action with its own argument type.
#[macro_export]
macro_rules! decl_continuation {
    {$(#[$meta:meta])* $(<web=$web_enable:block>)? $name:ident<$arg_type:ty>} => {
        $crate::contract::macros::paste! {
            $crate::contract::macros::web_api!($(#[$meta])* $name,$arg_type$(,$web_enable)*);
            $(#[$meta])*
            fn [<continue_ $name>](&self, _ctx: $crate::contract::Context, _args: $arg_type) -> $crate::contract::TxTmplIt {
                unimplemented!()
            }
            $(#[$meta])*
            fn $name() -> ::std::option::Option<::std::boxed::Box<dyn $crate::contract::actions::ErasedAction<Self>>> { ::std::option::Option::None }
        }
    };
}

/// Declare an optional guard in a contract interface: `decl_guard! { name }`.
/// Use `decl_guard! { cached name }` for a context-free cached clause.
/// Implement these with `#[guard] fn name(self, ctx: Context)` and
/// `#[guard(cached)] fn name(self)` respectively.
/// Custom backends use `decl_guard! { policy name<Backend> }` or
/// `decl_guard! { cached policy name<Backend> }` and `#[guard(policy)]`.
#[macro_export]
macro_rules! decl_guard {
    {
        $(#[$meta:meta])*
        cached policy $name:ident<$policy:ty>
    } => {
        $crate::contract::macros::paste! {
            $(#[$meta])*
            fn [<guard_ $name>](&self) -> $policy { unimplemented!(); }
            $(#[$meta])*
            fn $name() -> ::std::option::Option<$crate::contract::actions::Guard<Self>> {
                ::std::option::Option::None
            }
        }
    };
    {
        $(#[$meta:meta])*
        policy $name:ident<$policy:ty>
    } => {
        $crate::contract::macros::paste! {
            $(#[$meta])*
            fn [<guard_ $name>](&self, _ctx: $crate::contract::Context) -> $policy { unimplemented!(); }
            $(#[$meta])*
            fn $name() -> ::std::option::Option<$crate::contract::actions::Guard<Self>> {
                ::std::option::Option::None
            }
        }
    };
    {
        $(#[$meta:meta])*
        cached $name:ident
    } => {
        $crate::contract::macros::paste! {
            $(#[$meta])*
            fn [<guard_ $name>](&self) -> $crate::sapio_base::Clause {
                unimplemented!();
            }
            $(#[$meta])*
            fn $name() -> ::std::option::Option<$crate::contract::actions::Guard<Self>> {
                ::std::option::Option::None
            }
        }
    };
    {
        $(#[$meta:meta])*
        $name:ident} => {
            $crate::contract::macros::paste!{
                $(#[$meta])*
                fn [<guard_ $name>](&self, _ctx:$crate::contract::Context) -> $crate::sapio_base::Clause {
                    unimplemented!();
                }
                $(#[$meta])*
                fn $name() -> ::std::option::Option<$crate::contract::actions::Guard<Self>> {
                    ::std::option::Option::None
                }
            }
     };
}

/// declares a compile_if function for a trait interface.
#[macro_export]
macro_rules! decl_compile_if {
    {
        $(#[$meta:meta])*
        $name:ident
    } => {
            $crate::contract::macros::paste!{
                $(#[$meta])*
                fn [<compile_if_ $name>](&self, _ctx: $crate::contract::Context) -> $crate::contract::actions::ConditionalCompileType {
                    unimplemented!()
                }
                $(#[$meta])*
                fn $name() -> ::std::option::Option<$crate::contract::actions::ConditionallyCompileIf<Self>> {
                    ::std::option::Option::None
                }
            }
     };
}
