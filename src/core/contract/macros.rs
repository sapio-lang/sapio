/// The declare macro is used to declare the list of pathways in a Contract trait impl.
/// formats for calling are:
/// ```
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
        const THEN_FNS: &'a [fn() -> Option<$crate::contract::actions::ThenFunc<'a, Self>>] = &[$($a,)*];
    };
    [state $i:ty]  => {
        type StatefulArguments = $i;
    };

    [state]  => {
        #[cfg(feature = "nightly")]
        type StatefulArguments = ();
        #[cfg(not(feature = "nightly"))]
        type StatefulArguments;
    };
    {updatable<$($i:ty)?> $(,$a:expr)*} => {
        const FINISH_OR_FUNCS: &'a [fn() -> Option<$crate::contract::actions::FinishOrFunc<'a, Self, Self::StatefulArguments>>] = &[$($a,)*];
        declare![state $($i)?];
    };
    {non updatable} => {
        #[cfg(not(feature = "nightly"))]
        declare![state ()];
    };
    {finish $(,$a:expr)*} => {
        const FINISH_FNS: &'a [fn() -> Option<$crate::contract::actions::Guard<Self>>] = &[$($a,)*];
    };


}

/// The then macro is used to define a `ThenFunc`
/// formats for calling are:
/// ```
/// /// A Guarded CTV Function
/// then!(name [guard_1, ... guard_n] |s| {/*Result<Box<Iterator<TransactionTemplate>>>*/} );
/// /// An Unguarded CTV Function
/// then!(name |s| {/*Result<Box<Iterator<TransactionTemplate>>>*/} );
/// /// Null Implementation
/// then!(name);
/// ```
#[macro_export]
macro_rules! then {
    {$name:ident $a:tt |$s:ident| $b:block } => {
        fn $name() -> Option<$crate::contract::actions::ThenFunc<'a, Self>> { Some($crate::contract::actions::ThenFunc{guard: &$a, func:|$s: &Self| $b})}
    };
    {$name:ident |$s:ident| $b:block } => { then!{$name [] |$s| $b } };

    {$name:ident} => {
        fn $name() -> Option<$crate::contract::actions::ThenFunc<'a, Self>> {None}
    };
}

/// The then macro is used to define a `FinishFunc` or a `FinishOrFunc`
/// formats for calling are:
/// ```
/// /// A Guarded CTV Function
/// finish!(name [guard_1, ... guard_n] |s| {/*Result<Box<Iterator<TransactionTemplate>>>*/} );
/// /// Null Implementation
/// then!(name);
/// ```
/// Unlike a `then!`, `finish!` must always have guards.
#[macro_export]
macro_rules! finish {
    {$name:ident $a:tt |$s:ident, $o:ident| $b:block } => {
        fn $name() -> Option<$crate::contract::actions::FinishOrFunc<'a, Self, Args>>{
            Some($crate::contract::actions::FinishOrFunc{guard: &$a, func: |$s: &Self, $o: Option<&_>| $b} .into())
        }
    };
    {$name:ident $a:tt} => {
        finish!($name $a |s, o| { Ok(Box::new(std::iter::empty()))});
    };
}

/// The guard macro is used to define a `Guard`. Guards may be cached or uncached.
/// formats for calling are:
/// ```
/// guard!(name |s| {/*Clause*/})
/// /// The guard should only be invoked once
/// guard!(cached name |s| {/*Clause*/})
/// ```
#[macro_export]
macro_rules! guard {
    {$name:ident} => {
            fn $name() -> Option<$crate::contract::actions::Guard<Self>> {
                None
            }
     };
    {$name:ident |$s:ident| $b:block} => {
            fn $name() -> Option<$crate::contract::actions::Guard<Self>> {
                Some($crate::contract::actions::Guard::Fresh( |$s: &Self| $b))
            }
        };
    {cached $name:ident |$s:ident| $b:block} => {
            fn $name() -> Option<$crate::contract::actions::Guard<Self>> {
                Some($crate::contract::actions::Guard::Cache( |$s: &Self| $b))
            }
        };
}
