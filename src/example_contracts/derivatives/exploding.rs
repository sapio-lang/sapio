use super::*;
struct ExplodingOption<T> {
    party_one: Amount,
    party_two: Amount,
    key_p1: bitcoin::Address,
    key_p2: bitcoin::Address,
    key_p2_pk: Clause,
    opt: T,
    timeout: u32,
}

impl<T> ExplodingOption<T>
where
    GenericBet: TryFrom<T, Error = CompilationError>,
    T: Clone,
{
    then!(
        explodes | s,
        ctx | {
            ctx.template()
                .add_output(ctx.output(
                    s.party_one.into(),
                    &Compiled::from_address(s.key_p1.clone(), None),
                    None,
                )?)
                .add_output(ctx.output(
                    s.party_two.into(),
                    &Compiled::from_address(s.key_p2.clone(), None),
                    None,
                )?)
                .set_lock_time(s.timeout)
                .into()
        }
    );

    guard!(signed | s, ctx | { s.key_p2_pk.clone() });
    then!(
        stikes[Self::signed] | s,
        ctx | {
            ctx.template()
                .add_output(ctx.output(
                    (s.party_one + s.party_two).into(),
                    &GenericBet::try_from(s.opt.clone())?,
                    None,
                )?)
                .into()
        }
    );
}

struct UnderFundedExplodingOption<T> {
    party_one: Amount,
    party_two: Amount,
    key_p1: bitcoin::Address,
    key_p2: bitcoin::Address,
    opt: T,
    timeout: u32,
}

impl<T> UnderFundedExplodingOption<T>
where
    GenericBet: TryFrom<T, Error = CompilationError>,
    T: Clone,
{
    then!(
        explodes | s,
        ctx | {
            Ok(Box::new(std::iter::once(
                ctx.template()
                    .add_output(ctx.output(
                        s.party_one.into(),
                        &Compiled::from_address(s.key_p1.clone(), None),
                        None,
                    )?)
                    .set_lock_time(s.timeout)
                    .into(),
            )))
        }
    );

    then!(
        stikes | s,
        ctx | {
            ctx.template()
                .add_amount(s.party_two)
                .add_sequence(0)
                .add_output(ctx.output(
                    (s.party_one + s.party_two).into(),
                    &GenericBet::try_from(s.opt.clone())?,
                    None,
                )?)
                .into()
        }
    );
}
