use std::{marker::PhantomData, ops::ControlFlow};

pub mod breakability;
pub mod exclusivity;
pub mod fallibility;

pub use breakability::{Breakability, Breakable, Unbreakable};
pub use exclusivity::{Exclusivity, Immutable, Mutable, Pure};
pub use fallibility::{Fallibility, Fallible, Infallible};

pub(crate) type Output<T, E> = Result<
    <<E as Effect>::Breakability as Breakability>::Wrap<T>,
    <<E as Effect>::Fallibility as Fallibility>::Error,
>;

#[allow(dead_code)]
pub(crate) type CleanOutput<T, E> = <<E as Effect>::Fallibility as Fallibility>::CleanOutput<
    <<E as Effect>::Breakability as Breakability>::Wrap<T>,
>;

#[expect(clippy::unnecessary_wraps)]
pub(crate) fn pure<T: Send + 'static, E: Effect>(value: T) -> Output<T, E> {
    Ok(E::Breakability::cont(value))
}

pub(crate) fn extract<T: Send + 'static, U: Send + 'static, E: Effect>(
    value: Output<T, E>,
) -> Result<T, Output<U, E>> {
    match value.map(E::Breakability::unwrap) {
        Ok(ControlFlow::Continue(val)) => Ok(val),
        Ok(ControlFlow::Break(breaking)) => Err(Ok(breaking)),
        Err(err) => Err(Err(err)),
    }
}

pub(crate) fn error<T: Send + 'static, E: Effect, Err>(error: Err) -> Output<T, E>
where
    Err: Send + 'static + Into<<E::Fallibility as Fallibility>::Error>,
{
    Err(error.into())
}

#[allow(dead_code)]
pub(crate) fn clean<T: Send + 'static, E: Effect>(value: Output<T, E>) -> CleanOutput<T, E> {
    E::Fallibility::clean_output(value)
}

pub trait Effect: Sized + Send + Sync + 'static {
    type Exclusivity: Exclusivity
        + exclusivity::Contains<Pure>
        + exclusivity::Contains<Self::Exclusivity>;
    type Fallibility: Fallibility
        + fallibility::Contains<Infallible>
        + fallibility::Contains<Self::Fallibility>;
    type Breakability: Breakability
        + breakability::Contains<Unbreakable>
        + breakability::Contains<Self::Breakability>;
}

pub trait Partial: Send + Sync + 'static {
    type Use: Effect;
}

pub struct MakePartialImpl<E>(PhantomData<E>);
impl<E: Effect> Partial for MakePartialImpl<E> {
    type Use = E;
}

pub trait MakePartial: Effect {
    type Partial: Partial<Use = Self>;
}

impl<E: Effect> MakePartial for E {
    type Partial = MakePartialImpl<Self>;
}

#[doc(hidden)]
#[macro_export]
macro_rules! partial {
    ($ty:ty, $kind:ty $(, ($($tt:tt)*))?) => {
        impl $($($tt)*)? Partial for $ty {
            type Use = $kind;
        }
    };
}

pub trait Contains<E: Effect>:
    Effect<
        Exclusivity: exclusivity::Contains<E::Exclusivity>,
        Fallibility: fallibility::Contains<E::Fallibility>,
        Breakability: breakability::Contains<E::Breakability>,
    >
{
}

impl<Outer, Inner> Contains<Inner> for Outer
where
    Inner: Effect,
    Outer: Effect,
    Outer::Exclusivity: exclusivity::Contains<Inner::Exclusivity>,
    Outer::Fallibility: fallibility::Contains<Inner::Fallibility>,
    Outer::Breakability: breakability::Contains<Inner::Breakability>,
{
}

pub trait Apply<Other: Effect>: Effect {
    type And: Effect + Contains<Self>;
}

impl<A: Effect, O: Effect> Apply<O> for A
where
    A::Exclusivity: exclusivity::Apply<O::Exclusivity>,
    A::Fallibility: fallibility::Apply<O::Fallibility>,
    A::Breakability: breakability::Apply<O::Breakability>,
{
    type And = ApplyImpl<A, O>;
}

pub struct ApplyImpl<A, O>(PhantomData<(A, O)>);
impl<A: Effect, O: Effect> Effect for ApplyImpl<A, O>
where
    A::Exclusivity: exclusivity::Apply<O::Exclusivity>,
    A::Fallibility: fallibility::Apply<O::Fallibility>,
    A::Breakability: breakability::Apply<O::Breakability>,
{
    type Exclusivity = <A::Exclusivity as exclusivity::Apply<O::Exclusivity>>::And;
    type Fallibility = <A::Fallibility as fallibility::Apply<O::Fallibility>>::And;
    type Breakability = <A::Breakability as breakability::Apply<O::Breakability>>::And;
}
impl Effect for () {
    type Exclusivity = Pure;
    type Fallibility = Infallible;
    type Breakability = Unbreakable;
}

impl<E: Partial> Effect for (E,) {
    type Exclusivity = <E::Use as Effect>::Exclusivity;
    type Fallibility = <E::Use as Effect>::Fallibility;
    type Breakability = <E::Use as Effect>::Breakability;
}

impl<E: Partial, F: Partial> Effect for (E, F)
where
    E::Use: Apply<F::Use>,
{
    type Exclusivity = <<E::Use as Apply<F::Use>>::And as Effect>::Exclusivity;
    type Fallibility = <<E::Use as Apply<F::Use>>::And as Effect>::Fallibility;
    type Breakability = <<E::Use as Apply<F::Use>>::And as Effect>::Breakability;
}

impl<E1: Partial, E2: Partial, E3: Partial> Effect for (E1, E2, E3)
where
    E1::Use: Apply<E2::Use, And: Apply<E3::Use>>,
{
    type Exclusivity =
        <<<E1::Use as Apply<E2::Use>>::And as Apply<E3::Use>>::And as Effect>::Exclusivity;
    type Fallibility =
        <<<E1::Use as Apply<E2::Use>>::And as Apply<E3::Use>>::And as Effect>::Fallibility;
    type Breakability =
        <<<E1::Use as Apply<E2::Use>>::And as Apply<E3::Use>>::And as Effect>::Breakability;
}
