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
        const THEN_FNS: &'static [fn() -> Option<$crate::contract::actions::ThenFunc<Self>>] = &[$($a,)*];
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
        const FINISH_OR_FUNCS: &'static [fn() -> Option<$crate::contract::actions::FinishOrFunc<Self, Self::StatefulArguments>>] = &[$($a,)*];
        declare![state $($i)?];
    };
    {non updatable} => {
        #[cfg(not(feature = "nightly"))]
        declare![state ()];
    };
    {finish $(,$a:expr)*} => {
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
        $name:ident $a:tt |$s:ident, $ctx:ident| $b:block } => {
        $(#[$meta])*
        fn $name<'a>() -> Option<$crate::contract::actions::ThenFunc<Self>> { Some($crate::contract::actions::ThenFunc{guard: &$a, func:|$s: &Self, $ctx:&$crate::contract::Context| $b})}
    };
    {
        $(#[$meta:meta])*
        $name:ident |$s:ident, $ctx:ident| $b:block } => { then!{
            $(#[$meta])*
            $name [] |$s, $ctx| $b } };

    {
        $(#[$meta:meta])*
        $name:ident} => {
        $(#[$meta])*
        fn $name<'a>() -> Option<$crate::contract::actions::ThenFunc<Self>> {None}
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
        $name:ident $a:tt |$s:ident, $ctx:ident, $o:ident| $b:block } => {
        $(#[$meta])*
        fn $name<'a>() -> Option<$crate::contract::actions::FinishOrFunc<Self, Args>>{
            Some($crate::contract::actions::FinishOrFunc{guard: &$a, func: |$s: &Self, $ctx:&$crate::contract::Context, $o: Option<&_>| $b} .into())
        }
    };
    {
        $(#[$meta:meta])*
        $name:ident $a:tt} => {
        finish!(
            $(#[$meta])*
            $name $a |s, o| { Ok(Box::new(std::iter::empty()))});
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
            $(#[$meta])*
            fn $name() -> Option<$crate::contract::actions::Guard<Self>> {
                None
            }
     };
    {
        $(#[$meta:meta])*
        $name:ident |$s:ident, $ctx:ident| $b:block} => {
            $(#[$meta])*
            fn $name() -> Option<$crate::contract::actions::Guard<Self>> {
                Some($crate::contract::actions::Guard::Fresh( |$s: &Self, $ctx: &$crate::contract::Context| $b))
            }
        };
    {
        $(#[$meta:meta])*
        cached $name:ident |$s:ident, $ctx:ident| $b:block} => {
            $(#[$meta])*
            fn $name() -> Option<$crate::contract::actions::Guard<Self>> {
                Some($crate::contract::actions::Guard::Cache( |$s: &Self, $ctx:&$crate::contract::Context| $b))
            }
        };
}
