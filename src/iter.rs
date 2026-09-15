use std::{
    cmp::Ordering,
    collections::HashMap,
    hash::{BuildHasher, Hash},
    marker::PhantomData,
    ops::ControlFlow,
};

use futures::{Stream, StreamExt};

use crate::{
    effect::Effect,
    lazy::{Laze, Lazy, LazyOption, OptionLazyExt},
};

#[must_use = "a LazyIter value does nothing unless it is used"]
pub trait LazyIter<E: Effect>:
    IntoLazyIter<E, IntoItem = Self::Item> + Sized + Send + 'static
{
    type Item: Send + 'static;

    fn next(self) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = Self::Item>>, Self)>;

    fn size_hint(&self) -> (usize, Option<usize>) {
        (0, None)
    }

    fn map_lazy<T, L>(
        self,
        map: impl Fn(Self::Item) -> L + Clone + Send + 'static,
    ) -> impl LazyIter<E, Item = T>
    where
        L: Lazy<E, Output = T>,
        T: Send + 'static,
    {
        struct Map<I, M> {
            iter: I,
            map: M,
        }

        impl<E, I, T, U, L, M> LazyIter<E> for Map<I, M>
        where
            E: Effect,
            I: LazyIter<E, Item = T>,
            T: Send + 'static,
            U: Send + 'static,
            M: Fn(T) -> L + Clone + Send + 'static,
            L: Lazy<E, Output = U>,
        {
            type Item = U;
            fn next(self) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = U>>, Self)> {
                let Self { iter, map } = self;
                let map2 = map.clone();
                iter.next().then(|(next, iter)| {
                    Laze::just((next.map(move |lazy| lazy.then(map2)), Self { iter, map }))
                })
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                self.iter.size_hint()
            }
        }

        impl<E, I, T, U, L, M> IntoLazyIter<E> for Map<I, M>
        where
            E: Effect,
            I: LazyIter<E, Item = T>,
            T: Send + 'static,
            U: Send + 'static,
            M: Fn(T) -> L + Clone + Send + 'static,
            L: Lazy<E, Output = U>,
        {
            type IntoItem = U;
            fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
                self
            }
        }

        Map { iter: self, map }
    }

    fn map<T>(
        self,
        map: impl Fn(Self::Item) -> T + Clone + Send + 'static,
    ) -> impl LazyIter<E, Item = T>
    where
        T: Send + 'static,
    {
        self.map_lazy(move |value| Laze::just(map(value)))
    }

    fn take(self, count: usize) -> impl LazyIter<E, Item = Self::Item> {
        struct Take<I> {
            iter: I,
            count: usize,
        }

        impl<E, I, T> LazyIter<E> for Take<I>
        where
            E: Effect,
            I: LazyIter<E, Item = T>,
            T: Send + 'static,
        {
            type Item = T;

            fn next(
                self,
            ) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = Self::Item>>, Self)>
            {
                let Self { iter, count } = self;
                if let Some(count) = count.checked_sub(1) {
                    iter.next()
                        .map(move |(next, iter)| (next, Self { iter, count }))
                        .left()
                } else {
                    Laze::just((None, Self { iter, count: 0 })).right()
                }
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                let (low, high) = self.iter.size_hint();
                (
                    low.min(self.count),
                    Some(high.map_or_else(|| self.count, |high| high.min(self.count))),
                )
            }

            fn take(self, count: usize) -> impl LazyIter<E, Item = Self::Item> {
                Self {
                    count: self.count.min(count),
                    ..self
                }
            }
        }

        impl<E, I, T> IntoLazyIter<E> for Take<I>
        where
            E: Effect,
            I: LazyIter<E, Item = T>,
            T: Send + 'static,
        {
            type IntoItem = T;
            fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
                self
            }
        }

        Take { iter: self, count }
    }

    fn skip(self, count: usize) -> impl LazyIter<E, Item = Self::Item> {
        struct Skip<I> {
            iter: I,
            count: usize,
        }

        impl<E, I, T> LazyIter<E> for Skip<I>
        where
            E: Effect,
            I: LazyIter<E, Item = T>,
            T: Send + 'static,
        {
            type Item = T;
            fn next(
                self,
            ) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = Self::Item>>, Self)>
            {
                let Self { iter, count } = self;
                Laze::looping((iter, count), |(iter, count)| {
                    iter.next().map(move |(next, iter)| {
                        if count == 0 || next.is_none() {
                            ControlFlow::Break((next, Self { iter, count }))
                        } else {
                            ControlFlow::Continue((iter, count - 1))
                        }
                    })
                })
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                let (low, high) = self.iter.size_hint();
                let amount = self.count;

                (
                    low.saturating_sub(amount),
                    high.map(|high| high.saturating_sub(amount)),
                )
            }

            fn skip(self, amount: usize) -> impl LazyIter<E, Item = Self::Item> {
                Self {
                    count: self.count + amount,
                    ..self
                }
            }
        }

        impl<E, I, T> IntoLazyIter<E> for Skip<I>
        where
            E: Effect,
            I: LazyIter<E, Item = T>,
            T: Send + 'static,
        {
            type IntoItem = T;
            fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
                self
            }
        }

        Skip { iter: self, count }
    }

    fn flatten<T>(self) -> impl LazyIter<E, Item = T>
    where
        T: Send + 'static,
        Self::Item: IntoLazyIter<E, IntoItem = T>,
    {
        struct Flatten<I, In> {
            iter: I,
            inner: Option<In>,
        }

        impl<E, I, In, T> LazyIter<E> for Flatten<I, In>
        where
            E: Effect,
            I: LazyIter<E, Item = In>,
            In: LazyIter<E, Item = T>,
            T: Send + 'static,
        {
            type Item = T;

            fn next(
                self,
            ) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = Self::Item>>, Self)>
            {
                let Self { iter, inner } = self;
                Laze::looping((iter, inner), |(iter, inner)| {
                    if let Some(inner) = inner {
                        inner
                            .next()
                            .map(|(next, inner)| {
                                // We got a value, return it.
                                if next.is_some() {
                                    let inner = Some(inner);
                                    return ControlFlow::Break((next, Self { iter, inner }));
                                }

                                // The inner iterator ended, get a new one.
                                ControlFlow::Continue((iter, None))
                            })
                            .left()
                    } else {
                        iter.next()
                            .then(|(inner, iter)| {
                                // We're out of inner iterators, return None.
                                let Some(inner) = inner else {
                                    let flow =
                                        ControlFlow::Break((None, Self { iter, inner: None }));
                                    return Laze::just(flow).left();
                                };

                                // We got a new inner iterator, check it.
                                inner
                                    .map(move |inner| ControlFlow::Continue((iter, Some(inner))))
                                    .right()
                            })
                            .right()
                    }
                })
            }

            // flatten destroys our ability to reason about size
            fn size_hint(&self) -> (usize, Option<usize>) {
                (0, None)
            }
        }

        impl<E, I, In, T> IntoLazyIter<E> for Flatten<I, In>
        where
            E: Effect,
            I: LazyIter<E, Item = In>,
            In: LazyIter<E, Item = T>,
            T: Send + 'static,
        {
            type IntoItem = T;
            fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
                self
            }
        }

        fn flatten<E, T, I, In>(iter: I) -> impl LazyIter<E, Item = T>
        where
            E: Effect,
            T: Send + 'static,
            I: LazyIter<E, Item = In>,
            In: LazyIter<E, Item = T>,
        {
            Flatten { iter, inner: None }
        }

        flatten(self.map(IntoLazyIter::into_lazy_iter))
    }

    fn flat_map<T, I, F>(self, map: F) -> impl LazyIter<E, Item = T>
    where
        F: Fn(Self::Item) -> I + Clone + Send + 'static,
        I: IntoLazyIter<E, IntoItem = T>,
        T: Send + 'static,
    {
        self.map(map).flatten()
    }

    fn zip<T, I>(self, iter: I) -> impl LazyIter<E, Item = (Self::Item, T)>
    where
        I: IntoLazyIter<E, IntoItem = T>,
        T: Send + 'static,
    {
        struct Zip<IL, IR> {
            left: IL,
            right: IR,
        }

        impl<E, IL, IR, TL, TR> LazyIter<E> for Zip<IL, IR>
        where
            E: Effect,
            IL: LazyIter<E, Item = TL>,
            IR: LazyIter<E, Item = TR>,
            TL: Send + 'static,
            TR: Send + 'static,
        {
            type Item = (TL, TR);

            fn next(
                self,
            ) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = Self::Item>>, Self)>
            {
                let Self { left, right } = self;
                left.next().then(|(left_next, left)| {
                    if let Some(left_next) = left_next {
                        right
                            .next()
                            .map(|(right_next, right)| {
                                if let Some(right_next) = right_next {
                                    let next = left_next.then(|left_next| {
                                        right_next.map(|right_next| (left_next, right_next))
                                    });

                                    (Some(next), Self { left, right })
                                } else {
                                    (None, Self { left, right })
                                }
                            })
                            .left()
                    } else {
                        Laze::just((None, Self { left, right })).right()
                    }
                })
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                let (left_low, left_high) = self.left.size_hint();
                let (right_low, right_high) = self.right.size_hint();

                let low = left_low.min(right_low);
                let high = match (left_high, right_high) {
                    (None, None) => None,
                    (None, Some(right_high)) => Some(right_high),
                    (Some(left_high), None) => Some(left_high),
                    (Some(left_high), Some(right_high)) => Some(left_high.min(right_high)),
                };

                (low, high)
            }
        }

        impl<E, IL, IR, TL, TR> IntoLazyIter<E> for Zip<IL, IR>
        where
            E: Effect,
            IL: LazyIter<E, Item = TL>,
            IR: LazyIter<E, Item = TR>,
            TL: Send + 'static,
            TR: Send + 'static,
        {
            type IntoItem = (TL, TR);
            fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
                self
            }
        }

        Zip {
            left: self,
            right: iter.into_lazy_iter(),
        }
    }

    fn enumerate(self) -> impl LazyIter<E, Item = (usize, Self::Item)> {
        (0..).into_lazy_iter().zip(self)
    }

    fn cmp<I>(self, iter: I) -> impl Lazy<E, Output = Ordering>
    where
        I: IntoLazyIter<E, IntoItem = Self::Item>,
        Self::Item: Ord,
    {
        Laze::looping((self, iter.into_lazy_iter()), |(left, right)| {
            Laze::collect((left.next(), right.next())).then(
                move |((left_next, left), (right_next, right))| {
                    let ret = |ordering| Laze::just(ControlFlow::Break(ordering)).left();

                    match (left_next, right_next) {
                        (None, None) => ret(Ordering::Equal),
                        (Some(_), None) => ret(Ordering::Greater),
                        (None, Some(_)) => ret(Ordering::Less),
                        (Some(left_next), Some(right_next)) => {
                            Laze::collect((left_next, right_next))
                                .map(move |(left_next, right_next)| {
                                    let ordering = left_next.cmp(&right_next);
                                    if ordering.is_eq() {
                                        ControlFlow::Continue((left, right))
                                    } else {
                                        ControlFlow::Break(ordering)
                                    }
                                })
                                .right()
                        }
                    }
                },
            )
        })
    }

    fn fold_lazy<T, L, F>(self, value: T, mut f: F) -> impl Lazy<E, Output = (T, Self)>
    where
        F: FnMut(T, Self::Item) -> L + Send + 'static,
        L: Lazy<E, Output = T>,
        T: Send + 'static,
    {
        self.try_fold_lazy(value, move |acc, value| {
            f(acc, value).map(|value| Result::<_, !>::Ok(value))
        })
        .map(|(Ok(value), this)| (value, this))
    }

    fn fold<T, F>(self, value: T, mut f: F) -> impl Lazy<E, Output = (T, Self)>
    where
        F: FnMut(T, Self::Item) -> T + Send + 'static,
        T: Send + 'static,
    {
        self.fold_lazy(value, move |value, item| Laze::just(f(value, item)))
    }

    fn try_fold_lazy<T, Er, F, L>(
        self,
        value: T,
        f: F,
    ) -> impl Lazy<E, Output = (Result<T, Er>, Self)>
    where
        T: Send + 'static,
        Er: Send + 'static,
        F: FnMut(T, Self::Item) -> L + Send + 'static,
        L: Lazy<E, Output = Result<T, Er>>,
    {
        Laze::looping((value, f, self), |(value, mut f, iter)| {
            iter.next().then(move |(next, iter)| {
                let Some(next) = next else {
                    return Laze::just(ControlFlow::Break((Ok(value), iter))).left();
                };

                next.then(move |next_value| f(value, next_value).attach(f))
                    .map(move |(f, value)| match value {
                        Ok(value) => ControlFlow::Continue((value, f, iter)),
                        Err(err) => ControlFlow::Break((Err(err), iter)),
                    })
                    .right()
            })
        })
    }

    fn try_fold<T, Er, F>(self, value: T, mut f: F) -> impl Lazy<E, Output = (Result<T, Er>, Self)>
    where
        F: FnMut(T, Self::Item) -> Result<T, Er> + Send + 'static,
        T: Send + 'static,
        Er: Send + 'static,
    {
        self.try_fold_lazy(value, move |value, item| Laze::just(f(value, item)))
    }

    fn for_each_lazy<L, F>(self, mut f: F) -> impl Lazy<E, Output = Self>
    where
        F: FnMut(Self::Item) -> L + Send + 'static,
        L: Lazy<E, Output = ()>,
    {
        self.fold_lazy((), move |(), value| f(value))
            .map(|((), this)| this)
    }

    fn for_each<F>(self, mut f: F) -> impl Lazy<E, Output = Self>
    where
        F: FnMut(Self::Item) + Send + 'static,
    {
        self.for_each_lazy(move |item| {
            f(item);
            Laze::just(())
        })
    }

    fn any_lazy<L, F>(self, mut f: F) -> impl Lazy<E, Output = (bool, Self)>
    where
        F: FnMut(Self::Item) -> L + Send + 'static,
        L: Lazy<E, Output = bool>,
    {
        self.try_fold_lazy((), move |(), value| {
            f(value).map(|success| if success { Err(()) } else { Ok(()) })
        })
        .map(|(value, this)| (value.is_err(), this))
    }

    fn any<F>(self, mut f: F) -> impl Lazy<E, Output = (bool, Self)>
    where
        F: FnMut(Self::Item) -> bool + Send + 'static,
    {
        self.any_lazy(move |value| Laze::just(f(value)))
    }

    fn all_lazy<L, F>(self, mut f: F) -> impl Lazy<E, Output = (bool, Self)>
    where
        F: FnMut(Self::Item) -> L + Send + 'static,
        L: Lazy<E, Output = bool>,
    {
        self.try_fold_lazy((), move |(), value| {
            f(value).map(|success| if success { Ok(()) } else { Err(()) })
        })
        .map(|(value, this)| (value.is_ok(), this))
    }

    fn all<F>(self, mut f: F) -> impl Lazy<E, Output = (bool, Self)>
    where
        F: FnMut(Self::Item) -> bool + Send + 'static,
    {
        self.all_lazy(move |value| Laze::just(f(value)))
    }

    fn collect<I>(self) -> impl Lazy<E, Output = I>
    where
        I: FromLazyIter<E, Self::Item>,
    {
        I::from_lazy_iter(self).map(|(collection, _)| collection)
    }

    fn chain<I>(self, after: I) -> impl LazyIter<E, Item = Self::Item>
    where
        I: IntoLazyIter<E, IntoItem = Self::Item>,
    {
        struct Chain<F, S> {
            first: Option<F>,
            second: Option<S>,
        }

        impl<E, T, F, S> LazyIter<E> for Chain<F, S>
        where
            E: Effect,
            T: Send + 'static,
            F: LazyIter<E, Item = T>,
            S: LazyIter<E, Item = T>,
        {
            type Item = T;

            fn next(
                self,
            ) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = Self::Item>>, Self)>
            {
                let Self { first, second } = self;
                first
                    .map(LazyIter::next)
                    .transpose()
                    .and_then(|(next, first)| Laze::just(next.map(|next| (next, first))))
                    .then(|next| {
                        if let Some((next, first)) = next {
                            let first = Some(first);
                            Laze::just((Some(next.left()), Self { first, second })).left()
                        } else {
                            let first = None;
                            second
                                .map(LazyIter::next)
                                .transpose()
                                .and_then(|(next, second)| {
                                    Laze::just(next.map(|next| (next, second)))
                                })
                                .map(|next| {
                                    if let Some((next, second)) = next {
                                        let second = Some(second);
                                        (Some(next.right()), Self { first, second })
                                    } else {
                                        let second = None;
                                        (None, Self { first, second })
                                    }
                                })
                                .right()
                        }
                    })
            }
        }

        impl<E, T, F, S> IntoLazyIter<E> for Chain<F, S>
        where
            E: Effect,
            T: Send + 'static,
            F: LazyIter<E, Item = T>,
            S: LazyIter<E, Item = T>,
        {
            type IntoItem = T;
            fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
                self
            }
        }

        Chain {
            first: Some(self),
            second: Some(after.into_lazy_iter()),
        }
    }
}

pub struct LazeIter<E: Effect, T: Send + 'static>(PhantomData<(E, T)>);
impl<E: Effect, T: Send + 'static> LazeIter<E, T> {
    #[expect(clippy::should_implement_trait)] // can't implement it yet :(
    pub fn from_iter(
        iter: impl IntoIterator<Item = T, IntoIter: Send + 'static>,
    ) -> impl LazyIter<E, Item = T> {
        struct FromIter<I>(I);

        impl<E, T, I> LazyIter<E> for FromIter<I>
        where
            E: Effect,
            T: Send + 'static,
            I: Iterator<Item = T> + Send + 'static,
        {
            type Item = T;
            fn next(
                mut self,
            ) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = Self::Item>>, Self)>
            {
                let next = self.0.next();
                Laze::just((next.map(Laze::just), self))
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                self.0.size_hint()
            }
        }

        impl<E, T, I> IntoLazyIter<E> for FromIter<I>
        where
            E: Effect,
            T: Send + 'static,
            I: Iterator<Item = T> + Send + 'static,
        {
            type IntoItem = T;
            fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
                self
            }
        }

        FromIter(iter.into_iter())
    }

    pub fn from_stream(
        stream: impl Stream<Item = T> + Unpin + Send + 'static,
    ) -> impl LazyIter<E, Item = T> {
        struct FromStream<S>(S);
        impl<E, T, S> LazyIter<E> for FromStream<S>
        where
            E: Effect,
            T: Send + 'static,
            S: Stream<Item = T> + Unpin + Send + 'static,
        {
            type Item = T;
            fn next(
                mut self,
            ) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = Self::Item>>, Self)>
            {
                Laze::future(async move {
                    let next = self.0.next().await;
                    (next.map(Laze::just), self)
                })
            }

            fn size_hint(&self) -> (usize, Option<usize>) {
                self.0.size_hint()
            }
        }

        impl<E, T, S> IntoLazyIter<E> for FromStream<S>
        where
            E: Effect,
            T: Send + 'static,
            S: Stream<Item = T> + Unpin + Send + 'static,
        {
            type IntoItem = T;
            fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
                self
            }
        }

        FromStream(stream)
    }

    pub fn empty() -> impl LazyIter<E, Item = T> {
        Self::from_iter(std::iter::empty())
    }

    pub fn once(value: T) -> impl LazyIter<E, Item = T> {
        Self::from_iter(std::iter::once(value))
    }
}

pub trait IntoLazyIter<E: Effect>: Send + 'static {
    type IntoItem: Send + 'static;
    fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem>;
}

impl<E: Effect, T: Send + 'static> IntoLazyIter<E> for Vec<T> {
    type IntoItem = T;
    fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
        LazeIter::from_iter(self)
    }
}

impl<E: Effect, T: Send + 'static, const LEN: usize> IntoLazyIter<E> for [T; LEN] {
    type IntoItem = T;
    fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
        LazeIter::from_iter(self)
    }
}

impl<E, K, V, S> IntoLazyIter<E> for HashMap<K, V, S>
where
    E: Effect,
    K: Send + 'static,
    V: Send + 'static,
    S: Send + 'static,
{
    type IntoItem = (K, V);
    fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
        LazeIter::from_iter(self)
    }
}

impl<E, T> IntoLazyIter<E> for std::ops::Range<T>
where
    E: Effect,
    T: Send + 'static,
    Self: Iterator<Item = T>,
{
    type IntoItem = T;
    fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
        LazeIter::from_iter(self)
    }
}

impl<E, T> IntoLazyIter<E> for std::ops::RangeFrom<T>
where
    E: Effect,
    T: Send + 'static,
    Self: Iterator<Item = T>,
{
    type IntoItem = T;
    fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
        LazeIter::from_iter(self)
    }
}

impl<E, T> IntoLazyIter<E> for std::ops::RangeInclusive<T>
where
    E: Effect,
    T: Send + 'static,
    Self: Iterator<Item = T>,
{
    type IntoItem = T;
    fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
        LazeIter::from_iter(self)
    }
}

pub trait FromLazyIter<E: Effect, T: Send + 'static>: Sized + Send + 'static {
    fn from_lazy_iter<I>(iter: I) -> impl Lazy<E, Output = (Self, I)>
    where
        I: LazyIter<E, Item = T>;
}

impl<E: Effect, T: Send + 'static> FromLazyIter<E, T> for Vec<T> {
    fn from_lazy_iter<I>(iter: I) -> impl Lazy<E, Output = (Self, I)>
    where
        I: LazyIter<E, Item = T>,
    {
        let (low_bound, _) = iter.size_hint();
        let vec = Self::with_capacity(low_bound);
        iter.fold(vec, |mut vec, value| {
            vec.push(value);
            vec
        })
    }
}

impl<E, K, V, S> FromLazyIter<E, (K, V)> for HashMap<K, V, S>
where
    E: Effect,
    K: Eq + Hash + Send + 'static,
    V: Send + 'static,
    S: BuildHasher + Default + Send + 'static,
{
    fn from_lazy_iter<I>(iter: I) -> impl Lazy<E, Output = (Self, I)>
    where
        I: LazyIter<E, Item = (K, V)>,
    {
        let (low_bound, _) = iter.size_hint();
        let map = Self::with_capacity_and_hasher(low_bound, S::default());
        iter.fold(map, |mut map, (key, value)| {
            map.insert(key, value);
            map
        })
    }
}

impl<E, C, T, Err> FromLazyIter<E, Result<T, Err>> for Result<C, Err>
where
    E: Effect,
    C: FromLazyIter<E, T>,
    T: Send + 'static,
    Err: Send + 'static,
{
    fn from_lazy_iter<I>(iter: I) -> impl Lazy<E, Output = (Self, I)>
    where
        I: LazyIter<E, Item = Result<T, Err>>,
    {
        struct Shunt<I, Err> {
            iter: I,
            error: Option<Err>,
        }

        impl<E, I, T, Err> LazyIter<E> for Shunt<I, Err>
        where
            E: Effect,
            I: LazyIter<E, Item = Result<T, Err>>,
            T: Send + 'static,
            Err: Send + 'static,
        {
            type Item = T;
            fn next(
                self,
            ) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = Self::Item>>, Self)>
            {
                if self.error.is_some() {
                    Laze::just((None, self)).left()
                } else {
                    let Self { iter, error } = self;
                    iter.next()
                        .then(move |(item, iter)| {
                            let mut this = Self { iter, error };

                            item.transpose().map(|item| match item {
                                Some(Ok(item)) => (Some(Laze::just(item)), this),
                                Some(Err(err)) => {
                                    this.error = Some(err);
                                    (None, this)
                                }
                                None => (None, this),
                            })
                        })
                        .right()
                }
            }
        }

        impl<E, I, T, Err> IntoLazyIter<E> for Shunt<I, Err>
        where
            E: Effect,
            I: LazyIter<E, Item = Result<T, Err>>,
            T: Send + 'static,
            Err: Send + 'static,
        {
            type IntoItem = T;
            fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
                self
            }
        }

        C::from_lazy_iter(Shunt { iter, error: None }).map(|(collection, shunt)| {
            let Shunt { iter, error } = shunt;
            match error {
                Some(err) => (Err(err), iter),
                None => (Ok(collection), iter),
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use std::{
        cmp::Ordering,
        sync::{Arc, atomic::AtomicBool},
    };

    use crate::{
        effect::{self, Effect, Infallible, Unbreakable},
        global::GlobalState,
        iter::{IntoLazyIter, LazeIter, LazyIter},
        lazy::{Lazy, LazyContext},
    };

    async fn cmp<E, IL, IR, T>(left: IL, right: IR) -> std::cmp::Ordering
    where
        E: Effect<Fallibility = Infallible, Breakability = Unbreakable>,
        IL: IntoLazyIter<E, IntoItem = T>,
        IR: IntoLazyIter<E, IntoItem = T>,
        T: Ord + Send + 'static,
    {
        let state = GlobalState::default();
        let context = LazyContext { state: &state };

        let ordering = IntoLazyIter::<E>::into_lazy_iter(left).cmp(right);
        let ordering = ordering.get(context).await;

        ordering.into_value()
    }

    macro_rules! assert_iter_cmp_eq {
        ($left:expr, $right:expr $(,)?) => {
            assert_iter_cmp_eq!($left, $right, Ordering::Equal)
        };
        ($left:expr, $right:expr, $cmp:expr $(,)?) => {
            assert_iter_cmp_eq!($left, $right, $cmp, ())
        };
        ($left:expr, $right:expr, $cmp:expr, $effect:ty $(,)?) => {
            async {
                assert_eq!(cmp::<$effect, _, _, _>($left, $right).await, $cmp);
            }
        };
    }

    async fn run<E: Effect, L: Lazy<E, Output = T>, T: Send + 'static>(
        lazy: L,
    ) -> effect::CleanOutput<T, E> {
        let state = GlobalState::default();
        let context = LazyContext { state: &state };

        let output = lazy.get(context).await;
        output.into_value()
    }

    macro_rules! assert_lazy_eq {
        ($left:expr, $right:expr $(,)?) => {
            assert_lazy_eq!($left, $right, ())
        };
        ($left:expr, $right:expr, $effect:ty $(,)?) => {
            async {
                assert_eq!(run::<$effect, _, _>($left).await, $right);
            }
        };
    }

    #[tokio::test]
    async fn cmp_test() {
        assert_iter_cmp_eq!([], [0], Ordering::Less).await;
        assert_iter_cmp_eq!([] as [usize; 0], [], Ordering::Equal).await;
        assert_iter_cmp_eq!([0], [], Ordering::Greater).await;

        assert_iter_cmp_eq!([0], [1], Ordering::Less).await;
        assert_iter_cmp_eq!([0], [0], Ordering::Equal).await;
        assert_iter_cmp_eq!([1], [0], Ordering::Greater).await;

        assert_iter_cmp_eq!([0], [0, 0], Ordering::Less).await;
        assert_iter_cmp_eq!([0, 0], [0], Ordering::Greater).await;

        assert_iter_cmp_eq!([0, 0], [0, 1], Ordering::Less).await;
        assert_iter_cmp_eq!([0, 0], [0, 0], Ordering::Equal).await;
        assert_iter_cmp_eq!([0, 1], [0, 0], Ordering::Greater).await;
    }

    #[tokio::test]
    async fn take_simple() {
        assert_iter_cmp_eq!(LazeIter::from_iter(0..).take(5), [0, 1, 2, 3, 4]).await;
    }

    #[tokio::test]
    async fn take_twice() {
        assert_iter_cmp_eq!(LazeIter::from_iter(0..).take(5).take(2), [0, 1]).await;
        assert_iter_cmp_eq!(LazeIter::from_iter(0..).take(2).take(5), [0, 1]).await;
    }

    #[tokio::test]
    async fn take_empty() {
        assert_iter_cmp_eq!(LazeIter::<_, ()>::empty().take(5), []).await;
    }

    #[tokio::test]
    async fn skip_simple() {
        assert_iter_cmp_eq!([0, 1, 2, 3, 4].into_lazy_iter().skip(2), [2, 3, 4]).await;
    }

    #[tokio::test]
    async fn skip_twice() {
        assert_iter_cmp_eq!([0, 1, 2, 3, 4].into_lazy_iter().skip(2).skip(1), [3, 4]).await;
        assert_iter_cmp_eq!([0, 1, 2, 3, 4].into_lazy_iter().skip(1).skip(2), [3, 4]).await;
    }

    #[tokio::test]
    async fn skip_empty() {
        assert_iter_cmp_eq!(LazeIter::<_, ()>::empty().skip(1), []).await;
    }

    #[tokio::test]
    async fn collect_simple() {
        assert_lazy_eq!(
            [1, 2, 3].into_lazy_iter().collect::<Vec<_>>(),
            vec![1, 2, 3]
        )
        .await;
    }

    #[tokio::test]
    async fn collect_empty() {
        assert_lazy_eq!(LazeIter::empty().collect::<Vec<()>>(), vec![]).await;
    }

    #[tokio::test]
    async fn fold() {
        assert_lazy_eq!(
            [1, 2, 3]
                .into_lazy_iter()
                .fold(0, |acc, value| acc + value)
                .map(|(value, _)| value),
            6,
        )
        .await;
    }

    #[tokio::test]
    async fn collect_result_ok() {
        assert_lazy_eq!(
            [Ok(1), Ok(2), Ok(3)]
                .into_lazy_iter()
                .collect::<Result<Vec<_>, ()>>(),
            Ok(vec![1, 2, 3])
        )
        .await;
    }

    #[tokio::test]
    async fn collect_result_empty() {
        assert_lazy_eq!(
            [].into_lazy_iter().collect::<Result<Vec<()>, ()>>(),
            Ok(vec![])
        )
        .await;
    }

    #[tokio::test]
    async fn collect_result_err_single() {
        assert_lazy_eq!(
            [Ok(1), Err(2), Ok(3)]
                .into_lazy_iter()
                .collect::<Result<Vec<_>, _>>(),
            Err(2)
        )
        .await;
    }

    #[tokio::test]
    async fn collect_result_err_multi() {
        assert_lazy_eq!(
            [Err(1), Ok(2), Err(3)]
                .into_lazy_iter()
                .collect::<Result<Vec<_>, _>>(),
            Err(1)
        )
        .await;
    }

    #[tokio::test]
    async fn chain_empty() {
        assert_iter_cmp_eq!([(); 0].into_lazy_iter().chain([]), []).await;
    }

    #[tokio::test]
    async fn chain_simple() {
        assert_iter_cmp_eq!([1, 2].into_lazy_iter().chain([3, 4]), [1, 2, 3, 4]).await;
    }

    fn attach_marker<V: IntoLazyIter<(), IntoItem = bool>>(
        values: V,
        ran_marker: &Arc<AtomicBool>,
        value: bool,
    ) -> impl LazyIter<(), Item = bool> {
        let ran_marker = ran_marker.clone();
        let ran_marker = LazeIter::once(())
            .map(move |()| ran_marker.store(true, std::sync::atomic::Ordering::Relaxed))
            .map(move |()| value);

        values.into_lazy_iter().chain(ran_marker)
    }

    #[tokio::test]
    async fn all() {
        async fn assert_all_eq<V: IntoLazyIter<(), IntoItem = bool>>(values: V, value: bool) {
            assert_lazy_eq!(
                values
                    .into_lazy_iter()
                    .all(|value| value)
                    .map(|(value, _)| value),
                value
            )
            .await;
        }

        assert_all_eq([], true).await;
        assert_all_eq([true], true).await;
        assert_all_eq([false], false).await;

        let ran_marker = Arc::new(AtomicBool::new(false));

        assert_all_eq(attach_marker([false], &ran_marker, true), false).await;
        assert!(!ran_marker.load(std::sync::atomic::Ordering::Relaxed));

        assert_all_eq(attach_marker([true], &ran_marker, false), false).await;
        assert!(ran_marker.load(std::sync::atomic::Ordering::Relaxed));
    }

    #[tokio::test]
    async fn any() {
        async fn assert_any_eq<V: IntoLazyIter<(), IntoItem = bool>>(values: V, value: bool) {
            assert_lazy_eq!(
                values
                    .into_lazy_iter()
                    .any(|value| value)
                    .map(|(value, _)| value),
                value
            )
            .await;
        }

        assert_any_eq([], false).await;
        assert_any_eq([true], true).await;
        assert_any_eq([false], false).await;

        let ran_marker = Arc::new(AtomicBool::new(false));

        assert_any_eq(attach_marker([true], &ran_marker, false), true).await;
        assert!(!ran_marker.load(std::sync::atomic::Ordering::Relaxed));

        assert_any_eq(attach_marker([false], &ran_marker, true), true).await;
        assert!(ran_marker.load(std::sync::atomic::Ordering::Relaxed));
    }
}
