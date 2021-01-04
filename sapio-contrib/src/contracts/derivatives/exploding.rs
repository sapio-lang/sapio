use super::*;
pub trait Explodes: 'static + Sized {
    then!(explodes);
    then!(strikes);
}

impl<T> Contract for ExplodingOption<T>
where
    GenericBet: TryFrom<T, Error = CompilationError>,
    T: Clone + 'static,
{
    declare!(then, Self::explodes, Self::strikes);
    declare!(non updatable);
}

impl<T> Contract for UnderFundedExplodingOption<T>
where
    GenericBet: TryFrom<T, Error = CompilationError>,
    T: Clone + 'static,
{
    declare!(then, Self::explodes, Self::strikes);
    declare!(non updatable);
}

pub struct ExplodingOption<T: 'static> {
    party_one: Amount,
    party_two: Amount,
    key_p1: bitcoin::Address,
    key_p2: bitcoin::Address,
    key_p2_pk: Clause,
    opt: T,
    timeout: u32,
}

impl<T> ExplodingOption<T> {
    guard!(signed | s, ctx | { s.key_p2_pk.clone() });
}
impl<T> Explodes for ExplodingOption<T>
where
    GenericBet: TryFrom<T, Error = CompilationError>,
    T: Clone,
{
    then!(
        explodes | s,
        ctx | {
            ctx.template()
                .add_output(
                    s.party_one.into(),
                    &Compiled::from_address(s.key_p1.clone(), None),
                    None,
                )?
                .add_output(
                    s.party_two.into(),
                    &Compiled::from_address(s.key_p2.clone(), None),
                    None,
                )?
                .set_lock_time(s.timeout)
                .into()
        }
    );

    then!(
        strikes[Self::signed] | s,
        ctx | {
            ctx.template()
                .add_output(
                    (s.party_one + s.party_two).into(),
                    &GenericBet::try_from(s.opt.clone())?,
                    None,
                )?
                .into()
        }
    );
}

pub struct UnderFundedExplodingOption<T: 'static> {
    party_one: Amount,
    party_two: Amount,
    key_p1: bitcoin::Address,
    opt: T,
    timeout: u32,
}

impl<T> Explodes for UnderFundedExplodingOption<T>
where
    GenericBet: TryFrom<T, Error = CompilationError>,
    T: Clone,
{
    then!(
        explodes | s,
        ctx | {
            Ok(Box::new(std::iter::once(
                ctx.template()
                    .add_output(
                        s.party_one.into(),
                        &Compiled::from_address(s.key_p1.clone(), None),
                        None,
                    )?
                    .set_lock_time(s.timeout)
                    .into(),
            )))
        }
    );

    then!(
        strikes | s,
        ctx | {
            ctx.template()
                .add_amount(s.party_two)
                .add_sequence(0)
                .add_output(
                    (s.party_one + s.party_two).into(),
                    &GenericBet::try_from(s.opt.clone())?,
                    None,
                )?
                .into()
        }
    );
}
