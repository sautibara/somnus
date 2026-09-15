use std::marker::PhantomData;

use crate::{
    effect::{Effect, Partial},
    partial,
};

pub trait Exclusivity:
    Partial<Use = ExclusivityKind<Self>> + Sized + Send + Sync + 'static
{
    type Level: typenum::Unsigned
        + typenum::IsGreaterOrEqual<typenum::U0, Output = typenum::B1>
        + typenum::IsGreaterOrEqual<Self::Level, Output = typenum::B1>;
}

pub struct Pure(PhantomData<()>);
pub struct Immutable(PhantomData<()>);
pub struct Mutable(PhantomData<()>);

partial!(Pure, ExclusivityKind<Self>);
partial!(Immutable, ExclusivityKind<Self>);
partial!(Mutable, ExclusivityKind<Self>);

impl Exclusivity for Pure {
    type Level = typenum::U0;
}

impl Exclusivity for Immutable {
    type Level = typenum::U1;
}

impl Exclusivity for Mutable {
    type Level = typenum::U2;
}

pub struct ExclusivityKind<E: Exclusivity>(PhantomData<E>);
impl<E: Exclusivity> Effect for ExclusivityKind<E> {
    type Exclusivity = E;
    type Fallibility = <() as Effect>::Fallibility;
    type Breakability = <() as Effect>::Breakability;
}

pub trait Contains<E: Exclusivity>:
    Exclusivity<Level: typenum::IsGreaterOrEqual<E::Level, Output = typenum::B1>>
{
}

impl<Outer, Inner> Contains<Inner> for Outer
where
    Inner: Exclusivity,
    Outer: Exclusivity,
    Outer::Level: typenum::IsGreaterOrEqual<Inner::Level, Output = typenum::B1>,
{
}

pub trait Apply<Other: Exclusivity>: Exclusivity {
    type And: Exclusivity + Contains<Self>;
}

impl<E1: Exclusivity, E2: Exclusivity> Apply<E2> for E1
where
    E1::Level: typenum::Max<
            E2::Level,
            Output: typenum::Unsigned
                        + typenum::IsGreaterOrEqual<typenum::U0, Output = typenum::B1>
                        + typenum::IsGreaterOrEqual<E1::Level, Output = typenum::B1>
                        + typenum::IsGreaterOrEqual<
                <E1::Level as typenum::Max<E2::Level>>::Output,
                Output = typenum::B1,
            >,
        >,
{
    type And = ApplyImpl<E1, E2>;
}

pub struct ApplyImpl<E1, E2>(PhantomData<(E1, E2)>);
impl<E1: Exclusivity, E2: Exclusivity> Exclusivity for ApplyImpl<E1, E2>
where
    E1::Level: typenum::Max<
            E2::Level,
            Output: typenum::Unsigned
                        + typenum::IsGreaterOrEqual<typenum::U0, Output = typenum::B1>
                        + typenum::IsGreaterOrEqual<
                <E1::Level as typenum::Max<E2::Level>>::Output,
                Output = typenum::B1,
            >,
        >,
{
    type Level = <E1::Level as typenum::Max<E2::Level>>::Output;
}

impl<E1: Exclusivity, E2: Exclusivity> Partial for ApplyImpl<E1, E2>
where
    E1::Level: typenum::Max<
            E2::Level,
            Output: typenum::Unsigned
                        + typenum::IsGreaterOrEqual<typenum::U0, Output = typenum::B1>
                        + typenum::IsGreaterOrEqual<
                <E1::Level as typenum::Max<E2::Level>>::Output,
                Output = typenum::B1,
            >,
        >,
{
    type Use = ExclusivityKind<Self>;
}
