/// The def macro is used to define the list of pathways in a contract
#[macro_export]
macro_rules! def {
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
        def![state $($i)?];
    };
    {non updatable} => {
        #[cfg(not(feature = "nightly"))]
        def![state ()];
    };
    {finish $(,$a:expr)*} => {
        const FINISH_FNS: &'a [fn() -> Option<$crate::contract::actions::Guard<Self>>] = &[$($a,)*];
    };


}

/// The then macro is used to define a `ThenFunc`
#[macro_export]
macro_rules! then {
    {$name:ident $a:tt |$s:ident| $b:block } => {
        fn $name() -> Option<$crate::contract::actions::ThenFunc<'a, Self>> { Some($crate::contract::actions::ThenFunc(&$a, |$s: &Self| $b)) }
    };
    {$name:ident |$s:ident| $b:block } => { then!{$name [] |$s| $b } };

    {$name:ident} => {
        fn $name() -> Option<$crate::contract::actions::ThenFunc<'a, Self>> {None}
    };
}

/// The then macro is used to define a `FinishFunc` or a `FinishOrFunc`
#[macro_export]
macro_rules! finish {
    {$name:ident $a:tt |$s:ident, $o:ident| $b:block } => {
        fn $name() -> Option<$crate::contract::actions::FinishOrFunc<'a, Self, Args>>{
            Some($crate::contract::actions::FinishOrFuncNew(&$a, |$s: &Self, $o: Option<&_>| $b) .into())
        }
    };
    {$name:ident $a:tt} => {
        finish!($name $a |s, o| { Ok(Box::new(std::iter::empty()))});
    };
}

/// The guard macro is used to define a `Guard`. Guards may be cached or uncached.
#[macro_export]
macro_rules! guard {
    {$name:ident |$s:ident| $b:block} => {
            fn $name() -> Option<$crate::contract::actions::Guard<Self>> {
                Some($crate::contract::actions::Guard( |$s: &Self| $b, false))
            }
        };
    {cached $name:ident |$s:ident| $b:block} => {
            fn $name() -> Option<$crate::contract::actions::Guard<Self>> {
                Some($crate::contract::actions::Guard( |$s: &Self| $b, true))
            }
        };
}
