use std::marker::PhantomData;

use auditeur::namespaced_id::NamespacedIdRef;
use futures::{FutureExt, future::BoxFuture};
use thiserror::Error;

use crate::{
    effect::{
        Contains, Effect, Exclusivity, Fallibility, FallibilityContains, Fallible, Immutable,
        Mutable, Pure,
    },
    global::GlobalState,
    module::Module,
};

#[macro_export]
#[doc(hidden)]
macro_rules! run_lazy_inner {
    ($val:expr, $context:expr) => {
        match $val.get($context).await.extract() {
            Ok(value) => value,
            Err(value) => return value,
        }
    };
}

#[doc(hidden)]
#[derive(Clone, Copy)]
pub struct LazyContext<'a> {
    state: &'a GlobalState,
}

#[doc(hidden)]
pub struct LazyOutput<E: Effect, T: Send + 'static> {
    pub(crate) value: <E::Fallibility as Fallibility>::MapOutput<T>,
}

impl<E: Effect, T: Send + 'static> LazyOutput<E, T> {
    pub(crate) fn extract<O: Send + 'static>(self) -> Result<T, LazyOutput<E, O>> {
        match E::Fallibility::extract(self.value) {
            Ok(value) => Ok(value),
            Err(err) => Err(LazyOutput { value: err }),
        }
    }

    fn with_effect<E2>(self) -> LazyOutput<E2, T>
    where
        E2: Effect<Fallibility: FallibilityContains<E::Fallibility>>,
    {
        LazyOutput {
            value: E2::Fallibility::map_output(self.value),
        }
    }
}

#[must_use = "a Lazy value does nothing unless it is used"]
pub trait Lazy<E: Effect>: Sized + Send + 'static {
    type Output: Send + 'static;

    #[doc(hidden)]
    fn get(
        self,
        context: LazyContext<'_>,
    ) -> impl Future<Output = LazyOutput<E, Self::Output>> + Send + '_;

    fn boxed(self) -> BoxLazy<E, Self::Output> {
        BoxLazy {
            action: Box::new(|context| Box::pin(async move { self.get(context).await })),
            effect: PhantomData,
        }
    }

    fn then<T, L>(
        self,
        func: impl FnOnce(Self::Output) -> L + Send + 'static,
    ) -> impl Lazy<E, Output = T>
    where
        T: Send + 'static,
        L: Lazy<E, Output = T>,
    {
        struct Then<L, F> {
            lazy: L,
            then: F,
        }

        impl<L, F, L2, E, T, T2> Lazy<E> for Then<L, F>
        where
            L: Lazy<E, Output = T>,
            F: FnOnce(T) -> L2 + Send + 'static,
            L2: Lazy<E, Output = T2>,
            E: Effect,
            T: Send + 'static,
            T2: Send + 'static,
        {
            type Output = T2;
            async fn get(self, context: LazyContext<'_>) -> LazyOutput<E, Self::Output> {
                let value = self.lazy.get(context).await;
                let value = match value.extract() {
                    Ok(value) => value,
                    Err(err) => return err,
                };
                let lazy = (self.then)(value);
                lazy.get(context).await
            }
        }

        Then {
            lazy: self,
            then: func,
        }
    }

    fn map<T>(
        self,
        map: impl FnOnce(Self::Output) -> T + Send + 'static,
    ) -> impl Lazy<E, Output = T>
    where
        E: Contains<Pure>,
        T: Send + 'static,
    {
        self.then(|value| Laze::just(map(value)))
    }

    fn discard(self) -> impl Lazy<E, Output = ()>
    where
        E: Contains<Pure>,
    {
        self.map(|_| ())
    }

    #[must_use]
    fn with_effect(self, effect: E) -> Self {
        let _ = effect;
        self
    }

    fn using_effect<E2>(self) -> impl Lazy<E2, Output = Self::Output>
    where
        E2: Contains<E>,
    {
        struct AssertEffect<E1, L> {
            lazy: L,
            pd: PhantomData<E1>,
        }

        impl<E1: Effect, E2: Contains<E1>, L: Lazy<E1>> Lazy<E2> for AssertEffect<E1, L> {
            type Output = L::Output;
            fn get(
                self,
                context: LazyContext<'_>,
            ) -> impl Future<Output = LazyOutput<E2, Self::Output>> + Send + '_ {
                (self.lazy).get(context).map(LazyOutput::with_effect)
            }
        }

        AssertEffect {
            lazy: self,
            pd: PhantomData,
        }
    }

    fn overwrite_exclusivity<E2>(self) -> impl Lazy<E2, Output = Self::Output>
    where
        E2: Effect<Fallibility: FallibilityContains<E::Fallibility>>,
    {
        struct AssertEffect<E1, L> {
            lazy: L,
            pd: PhantomData<E1>,
        }

        impl<E1, E2, L> Lazy<E2> for AssertEffect<E1, L>
        where
            E1: Effect,
            E2: Effect<Fallibility: FallibilityContains<E1::Fallibility>>,
            L: Lazy<E1>,
        {
            type Output = L::Output;
            fn get(
                self,
                context: LazyContext<'_>,
            ) -> impl Future<Output = LazyOutput<E2, Self::Output>> + Send + '_ {
                (self.lazy).get(context).map(LazyOutput::with_effect)
            }
        }

        AssertEffect {
            lazy: self,
            pd: PhantomData,
        }
    }

    fn left<R>(self) -> Either<Self, R> {
        Either::Left(self)
    }

    fn right<L>(self) -> Either<L, Self> {
        Either::Right(self)
    }
}

type Action<E, T> =
    Box<dyn for<'a> FnOnce(LazyContext<'a>) -> BoxFuture<'a, LazyOutput<E, T>> + Send + 'static>;
pub struct BoxLazy<E: Effect, O: Send + 'static> {
    action: Action<E, O>,
    effect: PhantomData<E>,
}

impl<O: Send + 'static, E: Effect> Lazy<E> for BoxLazy<E, O> {
    type Output = O;

    async fn get(self, context: LazyContext<'_>) -> LazyOutput<E, Self::Output> {
        (self.action)(context).await
    }

    fn boxed(self) -> BoxLazy<E, Self::Output> {
        self
    }
}

pub struct Laze<T, E>(PhantomData<(T, E)>);
impl<T: Send + 'static, E: Effect> Laze<T, E> {
    pub fn just(value: T) -> impl Lazy<E, Output = T>
    where
        E: Contains<Pure>,
    {
        struct Just<T>(T);
        impl<T: Send + 'static, E: Effect> Lazy<E> for Just<T> {
            type Output = T;
            fn get(self, _: LazyContext<'_>) -> impl Future<Output = LazyOutput<E, T>> + Send + '_ {
                std::future::ready(LazyOutput {
                    value: E::Fallibility::pure(self.0),
                })
            }
        }
        Just(value)
    }

    pub fn lazy(func: impl FnOnce() -> T + Send + 'static) -> impl Lazy<E, Output = T> {
        struct Func<F>(F);
        impl<T, E, F> Lazy<E> for Func<F>
        where
            T: Send + 'static,
            E: Effect,
            F: FnOnce() -> T + Send + 'static,
        {
            type Output = T;
            async fn get(self, _: LazyContext<'_>) -> LazyOutput<E, Self::Output> {
                LazyOutput {
                    value: E::Fallibility::pure((self.0)()),
                }
            }
        }
        Func(func)
    }

    pub fn future(fut: impl Future<Output = T> + Send + 'static) -> impl Lazy<E, Output = T>
    where
        E: Contains<Pure>,
    {
        struct LazyFuture<F>(F);
        impl<T, F, E> Lazy<E> for LazyFuture<F>
        where
            T: Send + 'static,
            F: Future<Output = T> + Send + 'static,
            E: Effect,
        {
            type Output = T;
            fn get(self, _: LazyContext<'_>) -> impl Future<Output = LazyOutput<E, T>> + Send + '_ {
                self.0.map(|value| LazyOutput {
                    value: E::Fallibility::pure(value),
                })
            }
        }
        LazyFuture(fut)
    }

    pub fn with<M: Module>(
        fut: impl for<'a> AsyncFnOnce<(&'a M,), CallOnceFuture: Send, Output = T> + Send + 'static,
    ) -> impl Lazy<E, Output = T>
    where
        E: Contains<(Immutable, Fallible<ModuleError>)>,
    {
        struct With<F, T, M> {
            fut: F,
            pd: PhantomData<(T, M)>,
        }

        impl<T, M, F, E> Lazy<E> for With<F, T, M>
        where
            T: Send + 'static,
            M: Module,
            F: for<'a> AsyncFnOnce<(&'a M,), CallOnceFuture: Send, Output = T> + Send + 'static,
            E: Effect + Contains<(Immutable, Fallible<ModuleError>)>,
        {
            type Output = T;
            async fn get(self, context: LazyContext<'_>) -> LazyOutput<E, T> {
                let Some(module) = context.state.modules.get(M::id()) else {
                    let res = Err(ModuleError::NotFound { id: M::id() });
                    let value = E::Fallibility::map_output(res);
                    return LazyOutput { value };
                };

                let module = module.read().await;
                let value = (self.fut)(&module).await;
                drop(module);

                LazyOutput {
                    value: E::Fallibility::pure(value),
                }
            }
        }

        With {
            fut,
            pd: PhantomData,
        }
    }

    pub fn with_mut<M: Module>(
        fut: impl for<'a> AsyncFnOnce<(&'a mut M,), CallOnceFuture: Send, Output = T> + Send + 'static,
    ) -> impl Lazy<E, Output = T>
    where
        E: Contains<(Mutable, Fallible<ModuleError>)>,
    {
        struct With<F, T, M> {
            fut: F,
            pd: PhantomData<(T, M)>,
        }

        impl<T, M, F, E> Lazy<E> for With<F, T, M>
        where
            T: Send + 'static,
            M: Module,
            F: for<'a> AsyncFnOnce<(&'a mut M,), CallOnceFuture: Send, Output = T> + Send + 'static,
            E: Contains<(Mutable, Fallible<ModuleError>)>,
        {
            type Output = T;
            async fn get(self, context: LazyContext<'_>) -> LazyOutput<E, T> {
                let Some(module) = context.state.modules.get(M::id()) else {
                    let res = Err(ModuleError::NotFound { id: M::id() });
                    let value = E::Fallibility::map_output(res);
                    return LazyOutput { value };
                };

                let mut module = module.write().await;
                let value = (self.fut)(&mut module).await;
                drop(module);

                LazyOutput {
                    value: E::Fallibility::pure(value),
                }
            }
        }

        With {
            fut,
            pd: PhantomData,
        }
    }
}

pub trait LazyOption<E: Effect>: Lazy<E, Output = Option<Self::Inner>> {
    type Inner: Send + 'static;

    fn and_then<T, L>(
        self,
        func: impl FnOnce(Self::Inner) -> L + Send + 'static,
    ) -> impl Lazy<E, Output = Option<T>>
    where
        T: Send + 'static,
        L: Lazy<E, Output = Option<T>>,
    {
        struct AndThen<L, F> {
            lazy: L,
            then: F,
        }

        impl<L, F, L2, E, T, T2> Lazy<E> for AndThen<L, F>
        where
            L: Lazy<E, Output = Option<T>>,
            F: FnOnce(T) -> L2 + Send + 'static,
            L2: Lazy<E, Output = Option<T2>>,
            E: Effect,
            T: Send + 'static,
            T2: Send + 'static,
        {
            type Output = Option<T2>;
            async fn get(self, context: LazyContext<'_>) -> LazyOutput<E, Option<T2>> {
                let Some(value) = run_lazy_inner!(self.lazy, context) else {
                    return LazyOutput {
                        value: E::Fallibility::pure(None),
                    };
                };
                let lazy = (self.then)(value);
                lazy.get(context).await
            }
        }

        AndThen {
            lazy: self,
            then: func,
        }
    }
}

impl<T, E, L> LazyOption<E> for L
where
    T: Send + 'static,
    E: Effect,
    L: Lazy<E, Output = Option<T>>,
{
    type Inner = T;
}

pub trait LazyResult<E: Effect>: Lazy<E, Output = Result<Self::Inner, Self::Error>> {
    type Inner: Send + 'static;
    type Error: Send + 'static;

    fn and_then<T, L>(
        self,
        func: impl FnOnce(Self::Inner) -> L + Send + 'static,
    ) -> impl Lazy<E, Output = Result<T, Self::Error>>
    where
        T: Send + 'static,
        L: Lazy<E, Output = Result<T, Self::Error>>,
    {
        struct AndThen<L, F> {
            lazy: L,
            then: F,
        }

        impl<L, F, L2, E, T, T2, Err> Lazy<E> for AndThen<L, F>
        where
            L: Lazy<E, Output = Result<T, Err>>,
            F: FnOnce(T) -> L2 + Send + 'static,
            L2: Lazy<E, Output = Result<T2, Err>>,
            E: Effect,
            T: Send + 'static,
            T2: Send + 'static,
            Err: Send + 'static,
        {
            type Output = Result<T2, Err>;
            async fn get(self, context: LazyContext<'_>) -> LazyOutput<E, Result<T2, Err>> {
                let value = match run_lazy_inner!(self.lazy, context) {
                    Ok(value) => value,
                    Err(err) => {
                        return LazyOutput {
                            value: E::Fallibility::pure(Err(err)),
                        };
                    }
                };
                let lazy = (self.then)(value);
                lazy.get(context).await
            }
        }

        AndThen {
            lazy: self,
            then: func,
        }
    }

    fn throw(self) -> impl Lazy<E, Output = Self::Inner>
    where
        E: Contains<Fallible<Self::Error>>,
    {
        struct CancelOnError<L>(L);

        impl<L, E, T, Err> Lazy<E> for CancelOnError<L>
        where
            L: Lazy<E, Output = Result<T, Err>>,
            T: Send + 'static,
            E: Contains<Fallible<Err>>,
            Err: Send + 'static,
        {
            type Output = T;
            async fn get(self, context: LazyContext<'_>) -> LazyOutput<E, Self::Output> {
                match run_lazy_inner!(self.0, context) {
                    Ok(value) => LazyOutput {
                        value: E::Fallibility::pure(value),
                    },
                    Err(err) => {
                        let err = Err(err);
                        let value = E::Fallibility::map_output(err);
                        LazyOutput { value }
                    }
                }
            }
        }

        CancelOnError(self)
    }
}

impl<T, Err, E, L> LazyResult<E> for L
where
    T: Send + 'static,
    Err: Send + 'static,
    E: Effect,
    L: Lazy<E, Output = Result<T, Err>>,
{
    type Inner = T;
    type Error = Err;
}

pub trait LazyRecursive<E: Effect>: Lazy<E, Output = Self::InnerLazy> {
    type InnerValue: Send + 'static;
    type InnerLazy: Lazy<E, Output = Self::InnerValue>;

    fn flatten(self) -> impl Lazy<E, Output = Self::InnerValue>
    where
        E: Contains<Pure>,
    {
        self.then(|lazy| lazy)
    }
}

impl<T, E, L, L2> LazyRecursive<E> for L
where
    T: Send + 'static,
    E: Effect,
    L: Lazy<E, Output = L2>,
    L2: Lazy<E, Output = T>,
{
    type InnerValue = T;
    type InnerLazy = L2;
}

pub trait LazyFallible<E: Exclusivity, Err: Send + 'static>: Lazy<(E, Fallible<Err>)> {
    fn catch<E2>(self) -> impl Lazy<E2, Output = Result<Self::Output, Err>>
    where
        E2: Effect<Exclusivity = E>,
    {
        struct Catch<E1, L> {
            lazy: L,
            pd: PhantomData<E1>,
        }

        impl<E1, E2, L, Err> Lazy<E2> for Catch<E1, L>
        where
            E1: Effect<Fallibility = Fallible<Err>>,
            E2: Effect,
            L: Lazy<E1>,
            Err: Send + 'static,
        {
            type Output = Result<L::Output, Err>;
            async fn get(self, context: LazyContext<'_>) -> LazyOutput<E2, Self::Output> {
                let value = self.lazy.get(context).await;
                match value.extract() {
                    Ok(value) => LazyOutput {
                        value: E2::Fallibility::pure(Ok(value)),
                    },
                    Err(err) => LazyOutput {
                        value: E2::Fallibility::pure(err.value),
                    },
                }
            }
        }

        Catch {
            lazy: self,
            pd: PhantomData,
        }
    }
}

impl<E, L, Err> LazyFallible<E, Err> for L
where
    E: Exclusivity,
    L: Lazy<(E, Fallible<Err>)>,
    Err: Send + 'static,
{
}

pub enum Either<L, R> {
    Left(L),
    Right(R),
}

impl<E, L, R> Lazy<E> for Either<L, R>
where
    E: Effect,
    L: Lazy<E>,
    R: Lazy<E, Output = L::Output>,
{
    type Output = L::Output;
    fn get(
        self,
        context: LazyContext<'_>,
    ) -> impl Future<Output = LazyOutput<E, Self::Output>> + Send + '_ {
        match self {
            Self::Left(left) => left.get(context).left_future(),
            Self::Right(right) => right.get(context).right_future(),
        }
    }
}

pub trait OptionLazyExt<E: Effect> {
    type Value: Send + 'static;

    fn transpose(self) -> impl Lazy<E, Output = Option<Self::Value>>;
}

impl<E, L, T> OptionLazyExt<E> for Option<L>
where
    E: Contains<Pure>,
    L: Lazy<E, Output = T>,
    T: Send + 'static,
{
    type Value = T;

    fn transpose(self) -> impl Lazy<E, Output = Option<Self::Value>> {
        match self {
            Some(lazy) => lazy.map(Some).left(),
            None => Laze::just(None).right(),
        }
    }
}

#[derive(Error, Debug)]
pub enum ModuleError {
    #[error("module with id '{id}' has not been loaded, but an action is requesting it")]
    NotFound { id: &'static NamespacedIdRef },
}

#[cfg(test)]
mod tests {
    use crate::{
        effect::{Effect, Fallibility, Fallible, Pure},
        global::GlobalState,
        lazy::{Laze, Lazy, LazyContext, LazyFallible, LazyResult},
    };

    async fn run<E: Effect, L: Lazy<E, Output = T>, T: Send + 'static>(
        lazy: L,
    ) -> <E::Fallibility as Fallibility>::MapOutput<T> {
        let state = GlobalState::default();
        let context = LazyContext { state: &state };

        let output = lazy.get(context).await;
        output.value
    }

    macro_rules! assert_lazy_eq {
        ($left:expr, $right:expr) => {
            assert_lazy_eq!(Pure => ($left, $right))
        };
        ($effect:ty => ($left:expr, $right:expr)) => {
            async {
                assert_eq!(run::<$effect, _, _>($left).await, $right);
            }
        };
    }

    #[tokio::test]
    async fn just() {
        assert_lazy_eq!(Laze::just(5), 5).await;
    }

    #[tokio::test]
    async fn lazy() {
        assert_lazy_eq!(Laze::lazy(|| 5), 5).await;
    }

    #[tokio::test]
    async fn future() {
        assert_lazy_eq!(Laze::future(async { 5 }), 5).await;
    }

    #[tokio::test]
    async fn map() {
        assert_lazy_eq!(Laze::just(5).map(|x| x + 1), 6).await;
    }

    #[tokio::test]
    async fn then() {
        assert_lazy_eq!(Laze::just(5).then(|x| Laze::just(x + 1)), 6).await;

        assert_lazy_eq!(
            Laze::future(async { 5 }).then(|x| Laze::future(async move { x + 1 })),
            6
        )
        .await;
    }

    #[tokio::test]
    async fn throw() {
        assert_lazy_eq!(Fallible::<()> => (
            Laze::just(Err(())).throw(),
            Result::<(), ()>::Err(())
        ))
        .await;
    }

    #[tokio::test]
    async fn then_fallible() {
        assert_lazy_eq!(Fallible::<()> => (
            Laze::just(()).then(|()| Laze::just(Err(())).throw()),
            Result::<(), ()>::Err(())
        ))
        .await;
    }

    #[tokio::test]
    async fn throw_catch() {
        assert_lazy_eq!(
            Laze::just(Err(())).throw().catch(),
            Result::<(), ()>::Err(())
        )
        .await;
    }

    #[tokio::test]
    async fn boxed() {
        assert_lazy_eq!(Laze::just(5).boxed(), 5).await;
    }
}
