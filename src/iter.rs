use std::{
    collections::HashMap,
    hash::{BuildHasher, Hash},
    marker::PhantomData,
};

use futures::{Stream, StreamExt};

use crate::{
    effect::{Contains, Effect, Fallibility, Pure},
    lazy::{Laze, Lazy, LazyOutput, OptionLazyExt},
    run_lazy_inner,
};

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
        E: Contains<Pure>,
        L: Lazy<E, Output = T>,
        T: Send + 'static,
    {
        struct Map<I, M> {
            iter: I,
            map: M,
        }

        impl<E, I, T, U, L, M> LazyIter<E> for Map<I, M>
        where
            E: Contains<Pure>,
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
            E: Contains<Pure>,
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
        E: Contains<Pure>,
        T: Send + 'static,
    {
        self.map_lazy(move |value| Laze::just(map(value)))
    }

    fn take(self, count: usize) -> impl LazyIter<E, Item = Self::Item>
    where
        E: Contains<Pure>,
    {
        struct Take<I> {
            iter: I,
            count: usize,
        }

        impl<E, I, T> LazyIter<E> for Take<I>
        where
            E: Contains<Pure>,
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
        }

        impl<E, I, T> IntoLazyIter<E> for Take<I>
        where
            E: Contains<Pure>,
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

    fn skip(self, amount: usize) -> impl LazyIter<E, Item = Self::Item>
    where
        E: Contains<Pure>,
    {
        struct SkipWhile<I, S> {
            iter: I,
            smuggle: S,
            amount: usize,
        }

        impl<E, I, T, S, L, L2> Lazy<E> for SkipWhile<I, S>
        where
            E: Effect,
            I: LazyIter<E, Item = T>,
            T: Send + 'static,
            S: Fn(I) -> L + Send + 'static,
            L: Lazy<E, Output = (Option<L2>, I)>,
            L2: Lazy<E, Output = T>,
        {
            type Output = (Option<L2>, Self);
            async fn get(
                mut self,
                context: crate::lazy::LazyContext<'_>,
            ) -> LazyOutput<E, Self::Output> {
                loop {
                    let (next, iter) = run_lazy_inner!((self.smuggle)(self.iter), context);
                    self.iter = iter;

                    if self.amount == 0 {
                        return LazyOutput {
                            value: E::Fallibility::pure((next, self)),
                        };
                    }

                    self.amount -= 1;
                }
            }
        }

        impl<E, I, T, S, L, L2> LazyIter<E> for SkipWhile<I, S>
        where
            E: Effect,
            I: LazyIter<E, Item = T>,
            T: Send + 'static,
            S: Fn(I) -> L + Send + 'static,
            L: Lazy<E, Output = (Option<L2>, I)>,
            L2: Lazy<E, Output = T>,
        {
            type Item = T;
            fn next(
                self,
            ) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = Self::Item>>, Self)>
            {
                self
            }
        }

        impl<E, I, T, S, L, L2> IntoLazyIter<E> for SkipWhile<I, S>
        where
            E: Effect,
            I: LazyIter<E, Item = T>,
            T: Send + 'static,
            S: Fn(I) -> L + Send + 'static,
            L: Lazy<E, Output = (Option<L2>, I)>,
            L2: Lazy<E, Output = T>,
        {
            type IntoItem = T;
            fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
                self
            }
        }

        SkipWhile {
            iter: self,
            amount,
            smuggle: |iter: Self| iter.next(),
        }
    }

    fn flatten<T>(self) -> impl LazyIter<E, Item = T>
    where
        E: Contains<Pure>,
        T: Send + 'static,
        Self::Item: IntoLazyIter<E, IntoItem = T>,
    {
        struct Flatten<I, In, F> {
            iter: I,
            inner: Option<In>,
            smuggle: F,
        }

        impl<E, I, In, F, FL, L, T> Lazy<E> for Flatten<I, In, F>
        where
            E: Effect,
            I: LazyIter<E, Item = In>,
            In: LazyIter<E, Item = T>,
            F: Fn(In) -> FL + Send + 'static,
            FL: Lazy<E, Output = (Option<L>, In)>,
            L: Lazy<E, Output = T>,
            T: Send + 'static,
        {
            type Output = (Option<L>, Self);
            async fn get(
                mut self,
                context: crate::lazy::LazyContext<'_>,
            ) -> crate::lazy::LazyOutput<E, Self::Output> {
                loop {
                    // We already have an inner iterator:
                    if let Some(inner_inner) = self.inner {
                        let lazy = (self.smuggle)(inner_inner);
                        let (value, inner_new) = run_lazy_inner!(lazy, context);
                        self.inner = Some(inner_new);

                        // We found a value; we can give it.
                        if value.is_some() {
                            return LazyOutput {
                                value: E::Fallibility::pure((value, self)),
                            };
                        }

                        // We didn't find a value; we have to keep going.
                        self.inner = None;
                    }

                    // We need a new inner iterator:
                    let lazy = self.iter.next();
                    let (inner_new, iter_new) = run_lazy_inner!(lazy, context);
                    self.iter = iter_new;

                    // Outer iterator is empty; we're done.
                    let Some(inner_new) = inner_new else {
                        return LazyOutput {
                            value: E::Fallibility::pure((None, self)),
                        };
                    };

                    let inner_new = run_lazy_inner!(inner_new, context);
                    self.inner = Some(inner_new);
                }
            }
        }

        impl<E, I, In, F, FL, L, T> LazyIter<E> for Flatten<I, In, F>
        where
            E: Effect,
            I: LazyIter<E, Item = In>,
            In: LazyIter<E, Item = T>,
            F: Fn(In) -> FL + Send + 'static,
            FL: Lazy<E, Output = (Option<L>, In)>,
            L: Lazy<E, Output = T>,
            T: Send + 'static,
        {
            type Item = T;
            fn next(
                self,
            ) -> impl Lazy<E, Output = (Option<impl Lazy<E, Output = Self::Item>>, Self)>
            {
                self
            }
        }

        impl<E, I, In, F, FL, L, T> IntoLazyIter<E> for Flatten<I, In, F>
        where
            E: Effect,
            I: LazyIter<E, Item = In>,
            In: LazyIter<E, Item = T>,
            F: Fn(In) -> FL + Send + 'static,
            FL: Lazy<E, Output = (Option<L>, In)>,
            L: Lazy<E, Output = T>,
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
            Flatten {
                iter,
                inner: None,
                smuggle: |inner: In| inner.next(),
            }
        }

        flatten(self.map(IntoLazyIter::into_lazy_iter))
    }

    fn flat_map<T, I, F>(self, map: F) -> impl LazyIter<E, Item = T>
    where
        E: Contains<Pure>,
        F: Fn(Self::Item) -> I + Clone + Send + 'static,
        I: IntoLazyIter<E, IntoItem = T>,
        T: Send + 'static,
    {
        self.map(map).flatten()
    }

    fn fold_lazy<T, L, F>(self, value: T, mut f: F) -> impl Lazy<E, Output = (T, Self)>
    where
        E: Contains<Pure>,
        F: FnMut(T, Self::Item) -> L + Send + 'static,
        L: Lazy<E, Output = T>,
        T: Send + 'static,
    {
        self.try_fold_lazy(value, move |acc, value| {
            f(acc, value).map(|value| Result::<_, std::convert::Infallible>::Ok(value))
        })
        .map(|(res, this)| match res {
            Ok(value) => (value, this),
            Err(_) => unreachable!(),
        })
    }

    fn fold<T, F>(self, value: T, mut f: F) -> impl Lazy<E, Output = (T, Self)>
    where
        E: Contains<Pure>,
        F: FnMut(T, Self::Item) -> T + Send + 'static,
        T: Send + 'static,
    {
        self.fold_lazy(value, move |value, item| Laze::just(f(value, item)))
    }

    fn for_each_lazy<L, F>(self, mut f: F) -> impl Lazy<E, Output = Self>
    where
        E: Contains<Pure>,
        F: FnMut(Self::Item) -> L + Send + 'static,
        L: Lazy<E, Output = ()>,
    {
        self.fold_lazy((), move |(), value| f(value))
            .map(|((), this)| this)
    }

    fn for_each<F>(self, mut f: F) -> impl Lazy<E, Output = Self>
    where
        E: Contains<Pure>,
        F: FnMut(Self::Item) + Send + 'static,
    {
        self.for_each_lazy(move |item| {
            f(item);
            Laze::just(())
        })
    }

    fn try_fold_lazy<T, Er, F, L>(
        self,
        value: T,
        f: F,
    ) -> impl Lazy<E, Output = (Result<T, Er>, Self)>
    where
        E: Contains<Pure>,
        T: Send + 'static,
        Er: Send + 'static,
        F: FnMut(T, Self::Item) -> L + Send + 'static,
        L: Lazy<E, Output = Result<T, Er>>,
    {
        struct Fold<I, F, U> {
            iter: I,
            func: F,
            value: U,
        }

        impl<E, I, F, L, T, U, Er> Lazy<E> for Fold<I, F, U>
        where
            E: Effect,
            I: LazyIter<E, Item = T>,
            F: FnMut(U, T) -> L + Send + 'static,
            L: Lazy<E, Output = Result<U, Er>>,
            T: Send + 'static,
            U: Send + 'static,
            Er: Send + 'static,
        {
            type Output = (Result<U, Er>, I);
            async fn get(
                self,
                context: crate::lazy::LazyContext<'_>,
            ) -> LazyOutput<E, Self::Output> {
                let Self {
                    mut iter,
                    mut func,
                    mut value,
                } = self;

                loop {
                    let (next, new_iter) = run_lazy_inner!(iter.next(), context);
                    iter = new_iter;

                    let Some(next) = next else {
                        break;
                    };

                    let t = run_lazy_inner!(next, context);
                    match run_lazy_inner!(func(value, t), context) {
                        Ok(new_value) => value = new_value,
                        Err(err) => {
                            return LazyOutput {
                                value: E::Fallibility::pure((Err(err), iter)),
                            };
                        }
                    }
                }

                LazyOutput {
                    value: E::Fallibility::pure((Ok(value), iter)),
                }
            }
        }

        Fold {
            iter: self,
            func: f,
            value,
        }
    }

    fn try_fold<T, Er, F>(self, value: T, mut f: F) -> impl Lazy<E, Output = (Result<T, Er>, Self)>
    where
        E: Contains<Pure>,
        F: FnMut(T, Self::Item) -> Result<T, Er> + Send + 'static,
        T: Send + 'static,
        Er: Send + 'static,
    {
        self.try_fold_lazy(value, move |value, item| Laze::just(f(value, item)))
    }

    fn collect<I>(self) -> impl Lazy<E, Output = I>
    where
        E: Contains<Pure>,
        I: FromLazyIter<E, Self::Item>,
    {
        I::from_lazy_iter(self).map(|(collection, _)| collection)
    }
}

pub struct LazeIter<E: Effect, T: Send + 'static>(PhantomData<(E, T)>);
impl<E: Effect, T: Send + 'static> LazeIter<E, T> {
    #[expect(clippy::should_implement_trait)] // can't implement it yet :(
    pub fn from_iter(
        iter: impl IntoIterator<Item = T, IntoIter: Send + 'static>,
    ) -> impl LazyIter<E, Item = T>
    where
        E: Contains<Pure>,
    {
        struct FromIter<I>(I);

        impl<E, T, I> LazyIter<E> for FromIter<I>
        where
            E: Contains<Pure>,
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
            E: Contains<Pure>,
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
    ) -> impl LazyIter<E, Item = T>
    where
        E: Contains<Pure>,
    {
        struct FromStream<S>(S);
        impl<E, T, S> LazyIter<E> for FromStream<S>
        where
            E: Contains<Pure>,
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
            E: Contains<Pure>,
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
}

pub trait IntoLazyIter<E: Effect>: Send + 'static {
    type IntoItem: Send + 'static;
    fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem>;
}

impl<E: Contains<Pure>, T: Send + 'static> IntoLazyIter<E> for Vec<T> {
    type IntoItem = T;
    fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
        LazeIter::from_iter(self)
    }
}

impl<E, K, V, S> IntoLazyIter<E> for HashMap<K, V, S>
where
    E: Contains<Pure>,
    K: Send + 'static,
    V: Send + 'static,
    S: Send + 'static,
{
    type IntoItem = (K, V);
    fn into_lazy_iter(self) -> impl LazyIter<E, Item = Self::IntoItem> {
        LazeIter::from_iter(self)
    }
}

pub trait FromLazyIter<E: Effect, T: Send + 'static>: Sized + Send + 'static {
    fn from_lazy_iter<I>(iter: I) -> impl Lazy<E, Output = (Self, I)>
    where
        I: LazyIter<E, Item = T>;
}

impl<E: Contains<Pure>, T: Send + 'static> FromLazyIter<E, T> for Vec<T> {
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
    E: Contains<Pure>,
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
    E: Contains<Pure>,
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
            E: Contains<Pure>,
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
            E: Contains<Pure>,
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
