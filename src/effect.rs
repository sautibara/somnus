use std::{marker::PhantomData, ops::ControlFlow};

// TODO: errors should probably be outside of breakability

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

// TODO: bring back Kind and Apply, implement Effect for each Kind and its respective trait,
// like EffectImpl<Kind, E>, then (E,) will use EffectImpl, (E, E) will use Apply

pub trait Effect: Sized + Send + Sync + 'static {
    type Exclusivity: Exclusivity
        + ExclusivityContains<Pure>
        + ExclusivityContains<Self::Exclusivity>;
    type Fallibility: Fallibility
        + FallibilityContains<Infallible>
        + FallibilityContains<Self::Fallibility>;
    type Breakability: Breakability
        + BreakabilityContains<Unbreakable>
        + BreakabilityContains<Self::Breakability>;
}

pub trait Exclusivity:
    Partial<Use = ExclusivityKind<Self>> + Sized + Send + Sync + 'static
{
    type Level: typenum::Unsigned
        + typenum::IsGreaterOrEqual<typenum::U0, Output = typenum::B1>
        + typenum::IsGreaterOrEqual<Self::Level, Output = typenum::B1>;
}

pub trait Fallibility:
    Partial<Use = FallibilityKind<Self>> + Sized + Send + Sync + 'static
{
    type Error: From<!> + Send + 'static;

    type CleanOutput<T: Send + 'static>: Send + 'static;
    fn clean_output<T: Send + 'static>(value: Result<T, Self::Error>) -> Self::CleanOutput<T>;
}

pub trait Breakability:
    Partial<Use = BreakabilityKind<Self>> + Sized + Send + Sync + 'static
{
    type Wrap<Output: Send + 'static>: Send + 'static;

    fn cont<T: Send + 'static>(value: T) -> Self::Wrap<T>;
    fn unwrap<T: Send + 'static, E: Send + 'static>(
        wrapped: Self::Wrap<T>,
    ) -> ControlFlow<Self::Wrap<E>, T>;
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

macro_rules! partial {
    ($ty:ty, $kind:ty $(, ($($tt:tt)*))?) => {
        impl $($($tt)*)? Partial for $ty {
            type Use = $kind;
        }
    };
}

pub struct Pure(PhantomData<()>);
pub struct Immutable(PhantomData<()>);
pub struct Mutable(PhantomData<()>);

partial!(Pure, ExclusivityKind<Self>);
partial!(Immutable, ExclusivityKind<Self>);
partial!(Mutable, ExclusivityKind<Self>);

pub struct Infallible(PhantomData<()>);
pub struct Fallible<E>(PhantomData<fn() -> E>);

partial!(Infallible, FallibilityKind<Self>);
partial!(Fallible<E>, FallibilityKind<Self>, (<E: From<!> + Send + 'static>));

pub struct Unbreakable(PhantomData<()>);
pub struct Breakable<B>(PhantomData<fn() -> B>);

partial!(Unbreakable, BreakabilityKind<Self>);
partial!(Breakable<B>, BreakabilityKind<Self>, (<B: Send + 'static>));

impl Exclusivity for Pure {
    type Level = typenum::U0;
}

impl Exclusivity for Immutable {
    type Level = typenum::U1;
}

impl Exclusivity for Mutable {
    type Level = typenum::U2;
}

impl Fallibility for Infallible {
    type Error = !;

    type CleanOutput<T: Send + 'static> = T;
    fn clean_output<T: Send + 'static>(value: Result<T, Self::Error>) -> Self::CleanOutput<T> {
        match value {
            Ok(value) => value,
            Err(never) => never,
        }
    }
}

impl<E: From<!> + Send + 'static> Fallibility for Fallible<E> {
    type Error = E;

    type CleanOutput<T: Send + 'static> = Result<T, Self::Error>;
    fn clean_output<T: Send + 'static>(value: Result<T, Self::Error>) -> Self::CleanOutput<T> {
        value
    }
}

impl Breakability for Unbreakable {
    type Wrap<Output: Send + 'static> = Output;

    fn cont<T: Send + 'static>(value: T) -> Self::Wrap<T> {
        value
    }

    fn unwrap<T: Send + 'static, E: Send + 'static>(
        wrapped: Self::Wrap<T>,
    ) -> ControlFlow<Self::Wrap<E>, T> {
        ControlFlow::Continue(wrapped)
    }
}

impl<B: Send + 'static> Breakability for Breakable<B> {
    type Wrap<Output: Send + 'static> = ControlFlow<B, Output>;

    fn cont<T: Send + 'static>(value: T) -> Self::Wrap<T> {
        ControlFlow::Continue(value)
    }

    fn unwrap<T: Send + 'static, E: Send + 'static>(
        wrapped: Self::Wrap<T>,
    ) -> ControlFlow<Self::Wrap<E>, T> {
        match wrapped {
            ControlFlow::Continue(value) => ControlFlow::Continue(value),
            ControlFlow::Break(value) => ControlFlow::Break(ControlFlow::Break(value)),
        }
    }
}

pub struct ExclusivityKind<E: Exclusivity>(PhantomData<E>);
impl<E: Exclusivity> Effect for ExclusivityKind<E> {
    type Exclusivity = E;
    type Fallibility = <() as Effect>::Fallibility;
    type Breakability = <() as Effect>::Breakability;
}

pub struct FallibilityKind<F: Fallibility>(PhantomData<F>);
impl<F: Fallibility> Effect for FallibilityKind<F> {
    type Exclusivity = <() as Effect>::Exclusivity;
    type Fallibility = F;
    type Breakability = <() as Effect>::Breakability;
}

pub struct BreakabilityKind<B: Breakability>(PhantomData<B>);
impl<B: Breakability + BreakabilityContains<Unbreakable> + BreakabilityContains<B>> Effect
    for BreakabilityKind<B>
{
    type Exclusivity = <() as Effect>::Exclusivity;
    type Fallibility = <() as Effect>::Fallibility;
    type Breakability = B;
}

pub trait ExclusivityContains<E: Exclusivity>:
    Exclusivity<Level: typenum::IsGreaterOrEqual<E::Level, Output = typenum::B1>>
{
}

impl<Outer, Inner> ExclusivityContains<Inner> for Outer
where
    Inner: Exclusivity,
    Outer: Exclusivity,
    Outer::Level: typenum::IsGreaterOrEqual<Inner::Level, Output = typenum::B1>,
{
}

pub trait FallibilityContains<F: Fallibility>: Fallibility<Error: From<F::Error>> {}
impl<Outer, Inner> FallibilityContains<Inner> for Outer
where
    Inner: Fallibility,
    Outer: Fallibility,
    Outer::Error: From<Inner::Error>,
{
}

pub trait BreakabilityContains<B: Breakability>: Breakability {
    fn map<T: Send + 'static>(value: B::Wrap<T>) -> Self::Wrap<T>;
}

pub trait Contains<E: Effect>:
    Effect<
        Exclusivity: ExclusivityContains<E::Exclusivity>,
        Fallibility: FallibilityContains<E::Fallibility>,
        Breakability: BreakabilityContains<E::Breakability>,
    >
{
}

impl<Outer, Inner> Contains<Inner> for Outer
where
    Inner: Effect,
    Outer: Effect,
    Outer::Exclusivity: ExclusivityContains<Inner::Exclusivity>,
    Outer::Fallibility: FallibilityContains<Inner::Fallibility>,
    Outer::Breakability: BreakabilityContains<Inner::Breakability>,
{
}

pub trait Apply<Other: Effect>: Effect {
    type And: Effect + Contains<Self>;
}

impl<A: Effect, O: Effect> Apply<O> for A
where
    A::Exclusivity: ApplyExclusivity<O::Exclusivity>,
    A::Fallibility: ApplyFalliblity<O::Fallibility>,
    A::Breakability: ApplyBreakability<O::Breakability>,
{
    type And = ApplyImpl<A, O>;
}

pub struct ApplyImpl<A, O>(PhantomData<(A, O)>);
impl<A: Effect, O: Effect> Effect for ApplyImpl<A, O>
where
    A::Exclusivity: ApplyExclusivity<O::Exclusivity>,
    A::Fallibility: ApplyFalliblity<O::Fallibility>,
    A::Breakability: ApplyBreakability<O::Breakability>,
{
    type Exclusivity = <A::Exclusivity as ApplyExclusivity<O::Exclusivity>>::And;
    type Fallibility = <A::Fallibility as ApplyFalliblity<O::Fallibility>>::And;
    type Breakability = <A::Breakability as ApplyBreakability<O::Breakability>>::And;
}

pub trait ApplyExclusivity<Other: Exclusivity>: Exclusivity {
    type And: Exclusivity + ExclusivityContains<Self>;
}

impl<E1: Exclusivity, E2: Exclusivity> ApplyExclusivity<E2> for E1
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
    type And = ApplyExclusivityImpl<E1, E2>;
}

pub struct ApplyExclusivityImpl<E1, E2>(PhantomData<(E1, E2)>);
impl<E1: Exclusivity, E2: Exclusivity> Exclusivity for ApplyExclusivityImpl<E1, E2>
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

impl<E1: Exclusivity, E2: Exclusivity> Partial for ApplyExclusivityImpl<E1, E2>
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

pub trait ApplyFalliblity<Other: Fallibility>: Fallibility {
    type And: Fallibility + FallibilityContains<Self>;
}

impl<F: Fallibility> ApplyFalliblity<F> for Infallible {
    type And = F;
}

impl<F: Fallibility, E: From<!> + Send + 'static> ApplyFalliblity<F> for Fallible<E> {
    type And = Self;
}

pub trait ApplyBreakability<Other: Breakability>: Breakability {
    type And: Breakability
        + BreakabilityContains<Self>
        + BreakabilityContains<Unbreakable>
        + BreakabilityContains<Self::And>;
}

impl<Other: Breakability + BreakabilityContains<Self> + BreakabilityContains<Other>>
    ApplyBreakability<Other> for Unbreakable
{
    type And = Other;
}

impl<B: Send + 'static> ApplyBreakability<Unbreakable> for Breakable<B> {
    type And = Self;
}

impl<BI: Send + 'static, BO: Send + 'static> ApplyBreakability<Breakable<BO>> for Breakable<BI> {
    type And = BreakabilityConcat<Self, Breakable<BO>>;
}

impl<
    BI: Breakability,
    BO: Breakability + ApplyBreakability<BOO> + BreakabilityContains<BO>,
    BOO: Breakability,
> ApplyBreakability<BOO> for BreakabilityConcat<BI, BO>
{
    type And = BreakabilityConcat<BI, <BO as ApplyBreakability<BOO>>::And>;
}

pub struct BreakabilityConcat<I, O>(PhantomData<(I, O)>);
partial!(
    BreakabilityConcat<BI, BO>,
    BreakabilityKind<Self>,
    (<BI: Breakability, BO: Breakability + BreakabilityContains<BO>>)
);

impl<BI: Breakability, BO: Breakability + BreakabilityContains<BO>> Breakability
    for BreakabilityConcat<BI, BO>
{
    type Wrap<Output: Send + 'static> = BO::Wrap<BI::Wrap<Output>>;

    fn cont<T: Send + 'static>(value: T) -> Self::Wrap<T> {
        BO::cont(BI::cont(value))
    }

    fn unwrap<T: Send + 'static, E: Send + 'static>(
        wrapped: Self::Wrap<T>,
    ) -> ControlFlow<Self::Wrap<E>, T> {
        match BO::unwrap(wrapped) {
            ControlFlow::Continue(value) => match BI::unwrap(value) {
                ControlFlow::Continue(value) => ControlFlow::Continue(value),
                ControlFlow::Break(value) => ControlFlow::Break(BO::cont(value)),
            },
            ControlFlow::Break(value) => ControlFlow::Break(value),
        }
    }
}

impl BreakabilityContains<Self> for Unbreakable {
    fn map<T: Send + 'static>(value: Self::Wrap<T>) -> Self::Wrap<T> {
        value
    }
}

impl<B: Send + 'static> BreakabilityContains<Self> for Breakable<B> {
    fn map<T: Send + 'static>(value: Self::Wrap<T>) -> Self::Wrap<T> {
        value
    }
}

impl<B: Send + 'static> BreakabilityContains<Unbreakable> for Breakable<B> {
    fn map<T: Send + 'static>(value: T) -> Self::Wrap<T> {
        Self::cont(value)
    }
}

// Base case 1: (BI, BO) always contains BI
impl<BI: Send + 'static, BO: Breakability + BreakabilityContains<BO>>
    BreakabilityContains<Breakable<BI>> for BreakabilityConcat<Breakable<BI>, BO>
{
    fn map<T: Send + 'static>(value: <Breakable<BI> as Breakability>::Wrap<T>) -> Self::Wrap<T> {
        BO::cont(value)
    }
}

// Base case 2: (BI, BO) always contains Unbreakable
impl<BI: Breakability, BO: Breakability + BreakabilityContains<BO>>
    BreakabilityContains<Unbreakable> for BreakabilityConcat<BI, BO>
{
    fn map<T: Send + 'static>(value: <Unbreakable as Breakability>::Wrap<T>) -> Self::Wrap<T> {
        BO::cont(BI::cont(value))
    }
}

// Recursive step: If the innermost level is the same, we could recurse on an outer level.
// More specifically, if BO contains BC, then (BI, BO) also contains (BI, BC).
impl<
    BI: Breakability,
    BO: Breakability + BreakabilityContains<BC> + BreakabilityContains<BO>,
    BC: Breakability + BreakabilityContains<BC>,
> BreakabilityContains<BreakabilityConcat<BI, BC>> for BreakabilityConcat<BI, BO>
{
    fn map<T: Send + 'static>(
        value: <BreakabilityConcat<BI, BC> as Breakability>::Wrap<T>,
    ) -> Self::Wrap<T> {
        match BC::unwrap(value) {
            ControlFlow::Continue(inner) => BO::cont(inner),
            ControlFlow::Break(inner) => <BO as BreakabilityContains<BC>>::map(inner),
        }
    }
}

pub trait BreakOut: Breakability {
    type Outer: Send + 'static;

    type Inner: Breakability + BreakabilityContains<Self::Inner>;
    fn catch<U: Send + 'static>(
        value: Self::Wrap<U>,
    ) -> ControlFlow<Self::Outer, <Self::Inner as Breakability>::Wrap<U>>;

    fn break_out<U: Send + 'static>(value: Self::Outer) -> Self::Wrap<U>;
}

impl<T: Send + 'static> BreakOut for Breakable<T> {
    type Outer = T;

    type Inner = Unbreakable;
    fn catch<U: Send + 'static>(value: Self::Wrap<U>) -> ControlFlow<T, U> {
        value
    }

    fn break_out<U: Send + 'static>(value: T) -> Self::Wrap<U> {
        ControlFlow::Break(value)
    }
}

impl<BI, BO> BreakOut for BreakabilityConcat<Breakable<BI>, Breakable<BO>>
where
    BI: Send + 'static,
    BO: Send + 'static,
{
    type Outer = BO;

    type Inner = Breakable<BI>;
    fn catch<U: Send + 'static>(value: Self::Wrap<U>) -> ControlFlow<BO, ControlFlow<BI, U>> {
        value
    }

    fn break_out<U: Send + 'static>(value: BO) -> Self::Wrap<U> {
        Breakable::<BO>::break_out(value)
    }
}

impl<T, BI, BO, BOO> BreakOut for BreakabilityConcat<BI, BreakabilityConcat<BO, BOO>>
where
    T: Send + 'static,
    BI: Breakability,
    BO: Breakability,
    BOO: Breakability + BreakabilityContains<BOO>,
    BreakabilityConcat<BO, BOO>: BreakOut<Outer = T>,
{
    type Outer = T;

    type Inner = BreakabilityConcat<BI, <BreakabilityConcat<BO, BOO> as BreakOut>::Inner>;
    fn catch<U: Send + 'static>(
        value: Self::Wrap<U>,
    ) -> ControlFlow<T, <Self::Inner as Breakability>::Wrap<U>> {
        <BreakabilityConcat<BO, BOO> as BreakOut>::catch(value)
    }

    fn break_out<U: Send + 'static>(value: T) -> Self::Wrap<U> {
        <BreakabilityConcat<BO, BOO> as BreakOut>::break_out(value)
    }
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

pub trait MakeBreak<T: Send + 'static>: Effect {
    type Make: Effect<
            Exclusivity = Self::Exclusivity,
            Fallibility = Self::Fallibility,
            Breakability: BreakOut<Outer = T, Inner = Self::Breakability>,
        >;
}

impl<T, E> MakeBreak<T> for E
where
    T: Send + 'static,
    E: Effect,
    E::Breakability:
        ApplyBreakability<Breakable<T>, And: BreakOut<Outer = T, Inner = E::Breakability>>,
{
    type Make = MakeBreakImpl<T, Self>;
}

pub struct MakeBreakImpl<T: Send + 'static, E: Effect>(PhantomData<fn() -> (T, E)>);
impl<T, E> Effect for MakeBreakImpl<T, E>
where
    T: Send + 'static,
    E: Effect,
    E::Breakability:
        ApplyBreakability<Breakable<T>, And: BreakOut<Outer = T, Inner = E::Breakability>>,
{
    type Exclusivity = E::Exclusivity;
    type Fallibility = E::Fallibility;
    type Breakability = <E::Breakability as ApplyBreakability<Breakable<T>>>::And;
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! assert_breakability_contains {
        ($test_name:ident, $outer:ty, $inner:ty $(,)?) => {
            const _: fn() = || {
                const fn assert<T: super::BreakabilityContains<$inner>>() {}
                assert::<$outer>();
            };
        };
    }

    macro_rules! assert_types_eq {
        ($test_name:ident, $left:ty, $right:ty $(,)?) => {
            const _: fn() = || {
                const fn assert<T>(
                    _left: std::marker::PhantomData<T>,
                    _right: std::marker::PhantomData<T>,
                ) {
                }

                assert(
                    std::marker::PhantomData::<$left>,
                    std::marker::PhantomData::<$right>,
                );
            };
        };
    }

    macro_rules! assert_breakability_apply_eq {
        ($test_name:ident, ($first:ty $(, $left:ty)*), $right:ty $(,)?) => {
            assert_breakability_apply_eq!(@ $test_name ($first) ($($left),*) $right);
        };
        (@ $test_name:ident ($inner:ty) () $right:ty) => {
            assert_types_eq!($test_name, $inner, $right);
        };
        (@ $test_name:ident ($inner:ty) ($next:ty $(, $left:ty)*) $right:ty) => {
            assert_breakability_apply_eq!(@
                $test_name
                (<$inner as ApplyBreakability<$next>>::And)
                ($($left),*)
                $right
            );
        };
    }

    assert_breakability_contains!(unbreakable_contains_self, Unbreakable, Unbreakable);
    assert_breakability_contains!(breakable_contains_unbreakable, Breakable<()>, Unbreakable);
    assert_breakability_contains!(breakable_contains_self, Breakable<()>, Breakable<()>);

    type BEnd<I, O> = BreakabilityConcat<Breakable<I>, Breakable<O>>;
    type BConcat<I, O> = BreakabilityConcat<Breakable<I>, O>;

    struct T1;
    struct T2;
    struct T3;

    assert_breakability_contains!(
        concat_contains_unbreakable,
        BEnd<(), ()>,
        Unbreakable,
    );

    assert_breakability_contains!(
        concat_contains_breakable_inner,
        BEnd<T1, T2>,
        Breakable<T1>,
    );

    assert_breakability_contains!(
        concat_contains_self,
        BEnd<T1, T2>,
        BEnd<T1, T2>,
    );

    assert_breakability_contains!(
        large_concat_contains_self,
        BConcat<T1, BEnd<T2, T3>>,
        BConcat<T1, BEnd<T2, T3>>,
    );

    assert_breakability_contains!(
        large_concat_contains_smaller,
        BConcat<T1, BEnd<T2, T3>>,
        BEnd<T1, T2>,
    );

    assert_breakability_contains!(
        large_concat_contains_single,
        BConcat<T1, BEnd<T2, T3>>,
        Breakable<T1>,
    );

    assert_breakability_contains!(
        large_concat_contains_unbreakable,
        BConcat<T1, BEnd<T2, T3>>,
        Unbreakable,
    );

    assert_breakability_apply_eq!(
        unbreakable_apply_self,
        (Unbreakable, Unbreakable),
        Unbreakable
    );

    assert_breakability_apply_eq!(
        breakable_apply_unbreakable,
        (Breakable<T1>, Unbreakable),
        Breakable<T1>
    );

    assert_breakability_apply_eq!(
        unbreakable_apply_breakable,
        (Unbreakable, Breakable<T1>),
        Breakable<T1>
    );

    assert_breakability_apply_eq!(
        breakable_apply_breakable,
        (Breakable<T1>, Breakable<T2>),
        BEnd<T1, T2>,
    );

    assert_breakability_apply_eq!(
        breakable_apply_breakable_apply_breakable,
        (Breakable<T1>, Breakable<T2>, Breakable<T3>),
        BConcat<T1, BEnd<T2, T3>>,
    );

    assert_breakability_apply_eq!(
        breakable_apply_breakable_apply_unbreakable,
        (Breakable<T1>, Breakable<T2>, Unbreakable),
        BEnd<T1, T2>,
    );
}
