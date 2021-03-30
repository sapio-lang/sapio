// Copyright Judica, Inc 2021
//
// This Source Code Form is subject to the terms of the Mozilla Public
//  License, v. 2.0. If a copy of the MPL was not distributed with this
//  file, You can obtain one at https://mozilla.org/MPL/2.0/.

//! macros for making defining Sapio contracts less verbose.

pub use paste::paste;

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
        const THEN_FNS: &'static [fn() -> Option<$crate::contract::actions::ThenFunc<'static, Self>>] = &[$($a,)*];
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
        const FINISH_OR_FUNCS: &'static [fn() -> Option<$crate::contract::actions::FinishOrFunc<'static, Self, Self::StatefulArguments>>] = &[$($a,)*];
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
/// then!(name [guard_1, ... guard_n] |s| {/*Result<Box<Iterator<TransactionTemplate>>>*/} );
/// /// An Unguarded CTV Function
/// then!(name |s| {/*Result<Box<Iterator<TransactionTemplate>>>*/} );
/// /// Null Implementation
/// then!(name);
/// ```
#[macro_export]
macro_rules! then {
    {
        $(#[$meta:meta])*
        $name:ident $conditional_compile_list:tt $guard_list:tt |$s:ident, $ctx:ident| $b:block
    } => {
        $(#[$meta])*
        fn $name<'a>() -> Option<$crate::contract::actions::ThenFunc<'a, Self>>{
            Some($crate::contract::actions::ThenFunc{
                guard: &$guard_list,
                conditional_compile_if: &$conditional_compile_list,
                func:|$s: &Self, $ctx:&$crate::contract::Context| $b
            })
        }
    };
    {
        $(#[$meta:meta])*
        $name:ident |$s:ident, $ctx:ident| $b:block
    } => {
        then!{
            $(#[$meta])*
            $name [] [] |$s, $ctx| $b }
    };

    {
        $(#[$meta:meta])*
        $name:ident $guard_list:tt |$s:ident, $ctx:ident| $b:block
    } => {
        then!{
            $(#[$meta])*
            $name [] $guard_list |$s, $ctx| $b }
    };

    {
        $(#[$meta:meta])*
        $name:ident} => {
        $(#[$meta])*
        fn $name<'a>() -> Option<$crate::contract::actions::ThenFunc<'a, Self>> {None}
    };
}

/// The then macro is used to define a `FinishFunc` or a `FinishOrFunc`
/// formats for calling are:
/// ```ignore
/// /// A Guarded CTV Function
/// finish!(name [guard_1, ... guard_n] |s| {/*Result<Box<Iterator<TransactionTemplate>>>*/} );
/// /// Null Implementation
/// then!(name);
/// ```
/// Unlike a `then!`, `finish!` must always have guards.
#[macro_export]
macro_rules! finish {
    {
        $(#[$meta:meta])*
        $name:ident $conditional_compile_list:tt $guard_list:tt |$s:ident, $ctx:ident, $o:ident| $b:block
    } => {
        $(#[$meta])*
        fn $name<'a>() -> Option<$crate::contract::actions::FinishOrFunc<'a, Self, Args>>{
            Some($crate::contract::actions::FinishOrFunc{
                conditional_compile_if: &$conditional_compile_list,
                guard: &$guard_list,
                func: |$s: &Self, $ctx:&$crate::contract::Context, $o: Option<&_>| $b} .into())
        }
    };
    {
        $(#[$meta:meta])*
        $name:ident $guard_list:tt |$s:ident, $ctx:ident, $o:ident| $b:block
    } => {
        finish!{
            $(#[$meta])*
            $name [] $guard_list |$s, $ctx, $o| $b
        }
    };
    {
        $(#[$meta:meta])*
        $name:ident $guard_list:tt
    } => {
        finish!(
            $(#[$meta])*
            $name [] $guard_list |s, o| { Ok(Box::new(std::iter::empty()))});
    };
}

/// The guard macro is used to define a `Guard`. Guards may be cached or uncached.
/// formats for calling are:
/// ```ignore
/// guard!(name |s| {/*Clause*/})
/// /// The guard should only be invoked once
/// guard!(cached name |s| {/*Clause*/})
/// ```
#[macro_export]
macro_rules! guard {
    {
        $(#[$meta:meta])*
        $name:ident} => {
            $crate::contract::macros::paste!{
                $(#[$meta])*
                fn [<GUARD_ $name>](&self, _ctx:&$crate::contract::Context) -> $crate::sapio_base::Clause {
                    unimplemented!();
                }
                $(#[$meta])*
                fn $name() -> Option<$crate::contract::actions::Guard<Self>> {
                    None
                }
            }
     };
    {
        $(#[$meta:meta])*
        fn $name:ident($s:ident, $ctx:ident) $b:block} => {
            $crate::contract::macros::paste!{
                fn [<GUARD_ $name>](&$s, $ctx:&$crate::contract::Context) -> $crate::sapio_base::Clause
                $b
                $(#[$meta])*
                    fn  $name() -> Option<$crate::contract::actions::Guard<Self>> {
                    Some($crate::contract::actions::Guard::Fresh(Self::[<GUARD_ $name>]))
                }

            }
        };
    {
        $(#[$meta:meta])*
        cached
        fn $name:ident($s:ident, $ctx:ident) $b:block} => {
            $crate::contract::macros::paste!{
                fn [<GUARD_ $name>](&$s, $ctx:&$crate::contract::Context) -> $crate::sapio_base::Clause
                $b

                $(#[$meta])*
                fn $name() -> Option<$crate::contract::actions::Guard<Self>> {
                    Some($crate::contract::actions::Guard::Cache(Self::[<GUARD_ $name>]))
                }
            }
        };
}

/// The compile_if macro is used to define a `ConditionallyCompileIf`.
/// formats for calling are:
/// ```ignore
/// compile_if!(name |s| {/*ConditionallyCompileType*/})
/// ```
#[macro_export]
macro_rules! compile_if {
    {
        $(#[$meta:meta])*
        $name:ident
    } => {
            $(#[$meta])*
            fn $name() -> Option<$crate::contract::actions::ConditionallyCompileIf<Self>> {
                None
            }
     };
    {
        $(#[$meta:meta])*
        fn $name:ident($s:ident, $ctx:ident) $b:block
    } => {
            $(#[$meta])*
            fn $name() -> Option<$crate::contract::actions::ConditionallyCompileIf<Self>> {
                Some($crate::contract::actions::ConditionallyCompileIf::Fresh( |$s: &Self, $ctx: &$crate::contract::Context| $b))
            }
        };
}
