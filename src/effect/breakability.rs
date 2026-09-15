use std::{marker::PhantomData, ops::ControlFlow};

use crate::{
    effect::{Effect, Partial},
    partial,
};

pub trait Breakability: Partial<Use = Kind<Self>> + Sized + Send + Sync + 'static {
    type Wrap<Output: Send + 'static>: Send + 'static;

    fn cont<T: Send + 'static>(value: T) -> Self::Wrap<T>;
    fn unwrap<T: Send + 'static, E: Send + 'static>(
        wrapped: Self::Wrap<T>,
    ) -> ControlFlow<Self::Wrap<E>, T>;
}

pub struct Unbreakable(PhantomData<()>);
pub struct Breakable<B>(PhantomData<fn() -> B>);

partial!(Unbreakable, Kind<Self>);
partial!(Breakable<B>, Kind<Self>, (<B: Send + 'static>));

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

pub struct Kind<B: Breakability>(PhantomData<B>);
impl<B: Breakability + Contains<Unbreakable> + Contains<B>> Effect for Kind<B> {
    type Exclusivity = <() as Effect>::Exclusivity;
    type Fallibility = <() as Effect>::Fallibility;
    type Breakability = B;
}

pub trait Contains<B: Breakability>: Breakability {
    fn map<T: Send + 'static>(value: B::Wrap<T>) -> Self::Wrap<T>;
}

pub trait Apply<Other: Breakability>: Breakability {
    type And: Breakability + Contains<Self> + Contains<Unbreakable> + Contains<Self::And>;
}

impl<Other: Breakability + Contains<Self> + Contains<Other>> Apply<Other> for Unbreakable {
    type And = Other;
}

impl<B: Send + 'static> Apply<Unbreakable> for Breakable<B> {
    type And = Self;
}

impl<BI: Send + 'static, BO: Send + 'static> Apply<Breakable<BO>> for Breakable<BI> {
    type And = Concat<Self, Breakable<BO>>;
}

impl<BI: Breakability, BO: Breakability + Apply<BOO> + Contains<BO>, BOO: Breakability> Apply<BOO>
    for Concat<BI, BO>
{
    type And = Concat<BI, <BO as Apply<BOO>>::And>;
}

pub struct Concat<I, O>(PhantomData<(I, O)>);
partial!(
    Concat<BI, BO>,
    Kind<Self>,
    (<BI: Breakability, BO: Breakability + Contains<BO>>)
);

impl<BI: Breakability, BO: Breakability + Contains<BO>> Breakability for Concat<BI, BO> {
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

impl Contains<Self> for Unbreakable {
    fn map<T: Send + 'static>(value: Self::Wrap<T>) -> Self::Wrap<T> {
        value
    }
}

impl<B: Send + 'static> Contains<Self> for Breakable<B> {
    fn map<T: Send + 'static>(value: Self::Wrap<T>) -> Self::Wrap<T> {
        value
    }
}

impl<B: Send + 'static> Contains<Unbreakable> for Breakable<B> {
    fn map<T: Send + 'static>(value: T) -> Self::Wrap<T> {
        Self::cont(value)
    }
}

// Base case 1: (BI, BO) always contains BI
impl<BI: Send + 'static, BO: Breakability + Contains<BO>> Contains<Breakable<BI>>
    for Concat<Breakable<BI>, BO>
{
    fn map<T: Send + 'static>(value: <Breakable<BI> as Breakability>::Wrap<T>) -> Self::Wrap<T> {
        BO::cont(value)
    }
}

// Base case 2: (BI, BO) always contains Unbreakable
impl<BI: Breakability, BO: Breakability + Contains<BO>> Contains<Unbreakable> for Concat<BI, BO> {
    fn map<T: Send + 'static>(value: <Unbreakable as Breakability>::Wrap<T>) -> Self::Wrap<T> {
        BO::cont(BI::cont(value))
    }
}

// Recursive step: If the innermost level is the same, we could recurse on an outer level.
// More specifically, if BO contains BC, then (BI, BO) also contains (BI, BC).
impl<
    BI: Breakability,
    BO: Breakability + Contains<BC> + Contains<BO>,
    BC: Breakability + Contains<BC>,
> Contains<Concat<BI, BC>> for Concat<BI, BO>
{
    fn map<T: Send + 'static>(value: <Concat<BI, BC> as Breakability>::Wrap<T>) -> Self::Wrap<T> {
        match BC::unwrap(value) {
            ControlFlow::Continue(inner) => BO::cont(inner),
            ControlFlow::Break(inner) => <BO as Contains<BC>>::map(inner),
        }
    }
}

pub trait BreakOut: Breakability {
    type Outer: Send + 'static;

    type Inner: Breakability + Contains<Self::Inner>;
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

impl<BI, BO> BreakOut for Concat<Breakable<BI>, Breakable<BO>>
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

impl<T, BI, BO, BOO> BreakOut for Concat<BI, Concat<BO, BOO>>
where
    T: Send + 'static,
    BI: Breakability,
    BO: Breakability,
    BOO: Breakability + Contains<BOO>,
    Concat<BO, BOO>: BreakOut<Outer = T>,
{
    type Outer = T;

    type Inner = Concat<BI, <Concat<BO, BOO> as BreakOut>::Inner>;
    fn catch<U: Send + 'static>(
        value: Self::Wrap<U>,
    ) -> ControlFlow<T, <Self::Inner as Breakability>::Wrap<U>> {
        <Concat<BO, BOO> as BreakOut>::catch(value)
    }

    fn break_out<U: Send + 'static>(value: T) -> Self::Wrap<U> {
        <Concat<BO, BOO> as BreakOut>::break_out(value)
    }
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
    E::Breakability: Apply<Breakable<T>, And: BreakOut<Outer = T, Inner = E::Breakability>>,
{
    type Make = MakeBreakImpl<T, Self>;
}

pub struct MakeBreakImpl<T: Send + 'static, E: Effect>(PhantomData<fn() -> (T, E)>);
impl<T, E> Effect for MakeBreakImpl<T, E>
where
    T: Send + 'static,
    E: Effect,
    E::Breakability: Apply<Breakable<T>, And: BreakOut<Outer = T, Inner = E::Breakability>>,
{
    type Exclusivity = E::Exclusivity;
    type Fallibility = E::Fallibility;
    type Breakability = <E::Breakability as Apply<Breakable<T>>>::And;
}

#[cfg(test)]
mod tests {
    use super::*;

    macro_rules! assert_breakability_contains {
        ($test_name:ident, $outer:ty, $inner:ty $(,)?) => {
            const _: fn() = || {
                const fn assert<T: super::Contains<$inner>>() {}
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
                (<$inner as Apply<$next>>::And)
                ($($left),*)
                $right
            );
        };
    }

    assert_breakability_contains!(unbreakable_contains_self, Unbreakable, Unbreakable);
    assert_breakability_contains!(breakable_contains_unbreakable, Breakable<()>, Unbreakable);
    assert_breakability_contains!(breakable_contains_self, Breakable<()>, Breakable<()>);

    type BEnd<I, O> = Concat<Breakable<I>, Breakable<O>>;
    type BConcat<I, O> = Concat<Breakable<I>, O>;

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
