// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! macros for making defining Sapio contracts less verbose.

use core::any::TypeId;
pub use paste::paste;
use schemars::schema::RootSchema;
use std::collections::BTreeMap;
use std::sync::Arc;
use std::sync::Mutex;

/// The declare macro is used to declare the list of pathways in a Contract trait impl.
/// formats for calling are:
/// ```ignore
/// declare!{then, a,...}
/// declare!{finish, a,...}
/// declare!{updatable<X>, a,...}
/// /// because of a quirk in stable rust, non updatable
/// /// is required if no updatable<X> declaration is made
/// /// nightly rust does not require this, but it is availble
/// /// for compatibility
/// declare!{non updatable}
/// ```
#[macro_export]
macro_rules! declare {
    {then $(,$a:expr)*} => {
        /// binds the list of `ThenFunc`'s to this impl.
        /// Any fn() which returns None is ignored (useful for type-level state machines)
        const THEN_FNS: &'static [fn() -> Option<$crate::contract::actions::ThenFuncAsFinishOrFunc<'static, Self>>] = &[$($a,)*];
    };
    [state $i:ty]  => {
        type StatefulArguments = $i;
    };

    [state]  => {
        /// Due to type system limitations, all `FinishOrFuncs` for a Contract type must share a
        /// parameter pack type. If Nightly, trait default types allowed.
        #[cfg(feature = "nightly")]
        type StatefulArguments = ();
        /// Due to type system limitations, all `FinishOrFuncs` for a Contract type must share a
        /// parameter pack type. If stable, no default type allowed.
        #[cfg(not(feature = "nightly"))]
        type StatefulArguments;
    };
    {updatable<$($i:ty)?> $(,$a:expr)*} => {
        /// binds the list of `FinishOrFunc`'s to this impl.
        /// Any fn() which returns None is ignored (useful for type-level state machines)
        const FINISH_OR_FUNCS: &'static [fn() -> Option<Box<dyn $crate::contract::actions::CallableAsFoF<Self, Self::StatefulArguments>>>] = &[$($a,)*];
        declare![state $($i)?];
    };
    {non updatable} => {
        #[cfg(not(feature = "nightly"))]
        declare![state ()];
    };
    {finish $(,$a:expr)*} => {
        /// binds the list of `Gurard`'s to this impl as unlocking conditions.
        /// `Guard`s only need to be bound if it is desired that they are
        /// sufficient to unlock funds, a `Guard` should not be bound if it is
        /// intended to be used with a `ThenFunc`.
        /// Any fn() which returns None is ignored (useful for type-level state machines)
        const FINISH_FNS: &'static [fn() -> Option<$crate::contract::actions::Guard<Self>>] = &[$($a,)*];
    };


}

/// The then macro is used to define a `ThenFunc`
/// formats for calling are:
/// ```ignore
/// /// A Guarded CTV Function
/// then!(guarded_by: [guard_1, ... guard_n] fn name(self, ctx) {/*Result<Box<Iterator<TransactionTemplate>>>*/} );
/// /// A Conditional CTV Function
/// then!(compile_if: [compile_if_1, ... compile_if_n] fn name(self, ctx) {/*Result<Box<Iterator<TransactionTemplate>>>*/} );
/// /// An Unguarded CTV Function
/// then!(fn name(self, ctx) {/*Result<Box<Iterator<TransactionTemplate>>>*/} );
/// /// Null Implementation
/// then!(name);
/// ```
#[macro_export]
macro_rules! decl_then {
    {
        $(#[$meta:meta])*
        $name:ident
    } => {

        $crate::contract::macros::paste!{

            $(#[$meta])*
            fn [<then_ $name>](&self, _ctx:$crate::contract::Context, _:$crate::contract::actions::ThenFuncTypeTag)-> $crate::contract::TxTmplIt
            {
                unimplemented!();
            }
            $(#[$meta])*
            fn $name<'a>() -> Option<$crate::contract::actions::ThenFuncAsFinishOrFunc<'a, Self>> {None}
        }
    };
}

lazy_static::lazy_static! {
static ref SCHEMA_MAP: Mutex<BTreeMap<TypeId, Arc<RootSchema>>> =
Mutex::new(BTreeMap::new());
}
/// `get_schema_for` returns a cached RootSchema for a given type.  this is
/// useful because we might expect to generate the same RootSchema many times,
/// and they can use a decent amount of memory.
pub fn get_schema_for<T: schemars::JsonSchema + 'static + Sized>(
) -> Arc<schemars::schema::RootSchema> {
    SCHEMA_MAP
        .lock()
        .unwrap()
        .entry(TypeId::of::<T>())
        .or_insert_with(|| Arc::new(schemars::schema_for!(T)))
        .clone()
}

/// Internal Helper for finish! macro, not to be used directly.
#[macro_export]
macro_rules! web_api {
    {$name:ident,$type:ty,{}} => {
        $crate::contract::macros::paste!{
            const [<CONTINUE_SCHEMA_FOR_ $name:upper >] : Option<&'static dyn Fn() -> std::sync::Arc<$crate::schemars::schema::RootSchema>> = Some(&|| $crate::contract::macros::get_schema_for::<$type>());
        }
    };
    {$name:ident,$type:ty} => {
        $crate::contract::macros::paste!{
            const [<CONTINUE_SCHEMA_FOR_ $name:upper >] : Option<&'static dyn Fn() -> std::sync::Arc<$crate::schemars::schema::RootSchema>> = None;
        }
    }
}
pub use web_api;

/// Generates a type tag for WebAPI Enabled/Disabled
#[macro_export]
macro_rules! is_web_api_type {
    (
        $b:block
    ) => {
        $crate::contract::actions::WebAPIEnabled
    };
    () => {
        $crate::contract::actions::WebAPIDisabled
    };
}
/// The finish macro is used to define a `FinishFunc` or a `FinishOrFunc`
/// formats for calling are:
/// ```ignore
/// /// A Guarded CTV Function
/// finish!(guarded_by: [guard_1, ... guard_n] fn name(self, ctx, o) {/*Result<Box<Iterator<TransactionTemplate>>>*/} );
/// /// A Conditional CTV Function
/// finish!(compile_if: [compile_if_1, ... compile_if_n] guarded_by: [guard_1, ..., guard_n] fn name(self, ctx, o) {/*Result<Box<Iterator<TransactionTemplate>>>*/} );
/// /// Null Implementation
/// finish!(name);
/// ```
/// Unlike a `then!`, `finish!` must always have guards.
#[macro_export]
macro_rules! decl_continuation {
    {
        $(#[$meta:meta])*
        $(<web=$web_enable:block>)?
        $name:ident<$arg_type:ty>
    } => {
        $crate::contract::macros::paste!{
            $crate::contract::macros::web_api!($name,$arg_type$(,$web_enable)*);
            $(#[$meta])*
            fn [<continue_ $name>](&self, _ctx:$crate::contract::Context, _o: $arg_type)-> $crate::contract::TxTmplIt
            {
                unimplemented!();
            }
            $(#[$meta])*
            fn $name<'a>() ->
            Option<Box<dyn
            $crate::contract::actions::CallableAsFoF<Self, <Self as $crate::contract::Contract>::StatefulArguments>>>
            {
                None
            }
        }
    };
}

/// The guard macro is used to define a `Guard`. Guards may be cached or uncached.
/// formats for calling are:
/// ```ignore
/// guard!(fn name(self, ctx) {/*Clause*/})
/// /// The guard should only be invoked once
/// guard!(cached fn name(self, ctx) {/*Clause*/})
/// ```
#[macro_export]
macro_rules! decl_guard {
    {
        $(#[$meta:meta])*
        $name:ident} => {
            $crate::contract::macros::paste!{
                $(#[$meta])*
                fn [<guard_ $name>](&self, _ctx:$crate::contract::Context) -> $crate::sapio_base::Clause {
                    unimplemented!();
                }
                $(#[$meta])*
                fn $name() -> Option<$crate::contract::actions::Guard<Self>> {
                    None
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
                fn [<compile_if $name>](&self, _ctx: $crate::contract::Context) -> $crate::contract::actions::ConditionalCompileType {
                    unimplemented!()
                }
                $(#[$meta])*
                fn $name() -> Option<$crate::contract::actions::ConditionallyCompileIf<Self>> {
                    None
                }
            }
     };
}
