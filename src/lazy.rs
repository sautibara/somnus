use std::{marker::PhantomData, ops::ControlFlow};

use auditeur::namespaced_id::NamespacedIdRef;
use futures::{FutureExt, future::BoxFuture};
use thiserror::Error;

use crate::effect::breakability::Contains as _;
use crate::{
    effect::{
        self, Apply, Breakability, Breakable, Contains, Effect, Fallibility, Fallible, Immutable,
        Infallible, MakePartial, Mutable, Partial, Pure, Unbreakable,
        breakability::{self, BreakOut, MakeBreak},
        fallibility,
    },
    global::GlobalState,
    module::Module,
};

// TODO: Lazy::looping - accepts an fn that takes in some Continue lazy
// - implements this by adding a new looping effect
// - may have multiple loops by stacking effects
// - continue lazy actually returns some break output that continues the loop
//   - can't have a recursive function because we can't use the lazy's output (since it breaks)

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
    pub(crate) state: &'a GlobalState,
}

#[doc(hidden)]
pub struct LazyOutput<E: Effect, T: Send + 'static> {
    value: effect::Output<T, E>,
}

impl<E: Effect, T: Send + 'static> LazyOutput<E, T> {
    pub(crate) fn pure(value: T) -> Self {
        Self {
            value: effect::pure::<T, E>(value),
        }
    }

    pub(crate) fn extract<O: Send + 'static>(self) -> Result<T, LazyOutput<E, O>> {
        match effect::extract::<T, O, E>(self.value) {
            Ok(value) => Ok(value),
            Err(err) => Err(LazyOutput { value: err }),
        }
    }

    pub fn into_value(self) -> T
    where
        E: Effect<Fallibility: Fallibility<Error = !>, Breakability = Unbreakable>,
    {
        match self.value {
            Ok(value) => value,
            Err(never) => never,
        }
    }

    fn with_effect<E2>(self) -> LazyOutput<E2, T>
    where
        E2: Effect<
                Fallibility: fallibility::Contains<E::Fallibility>,
                Breakability: breakability::Contains<E::Breakability>,
            >,
    {
        LazyOutput {
            value: self.value.map(E2::Breakability::map).map_err(Into::into),
        }
    }

    pub fn err<Er2: From<!> + Send + 'static>(err: Er2) -> Self
    where
        E: Contains<(Fallible<Er2>,)>,
    {
        Self {
            value: effect::error::<T, E, Er2>(err),
        }
    }

    pub const fn breaking(breaking: <E::Breakability as Breakability>::Wrap<T>) -> Self {
        Self {
            value: Ok(breaking),
        }
    }

    pub fn break_out<B>(value: B) -> Self
    where
        B: Send + 'static,
        E::Breakability: BreakOut<Outer = B>,
    {
        Self {
            value: Ok(E::Breakability::break_out(value)),
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
        T: Send + 'static,
    {
        self.then(|value| Laze::just(map(value)))
    }

    fn discard(self) -> impl Lazy<E, Output = ()> {
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
        E2: Effect<
                Fallibility: fallibility::Contains<E::Fallibility>,
                Breakability: breakability::Contains<E::Breakability>,
            >,
    {
        struct AssertEffect<E1, L> {
            lazy: L,
            pd: PhantomData<E1>,
        }

        impl<E1, E2, L> Lazy<E2> for AssertEffect<E1, L>
        where
            E1: Effect,
            E2: Effect<
                    Fallibility: fallibility::Contains<E1::Fallibility>,
                    Breakability: breakability::Contains<E1::Breakability>,
                >,
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

    fn attach<T: Send + 'static>(self, value: T) -> impl Lazy<E, Output = (T, Self::Output)> {
        self.map(|out| (value, out))
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
    pub fn just(value: T) -> impl Lazy<E, Output = T> {
        struct Just<T>(T);
        impl<T: Send + 'static, E: Effect> Lazy<E> for Just<T> {
            type Output = T;
            fn get(self, _: LazyContext<'_>) -> impl Future<Output = LazyOutput<E, T>> + Send + '_ {
                std::future::ready(LazyOutput::pure(self.0))
            }
        }
        Just(value)
    }

    pub fn blocking(func: impl FnOnce() -> T + Send + 'static) -> impl Lazy<E, Output = T> {
        struct Blocking<F>(F);
        impl<T, E, F> Lazy<E> for Blocking<F>
        where
            T: Send + 'static,
            E: Effect,
            F: FnOnce() -> T + Send + 'static,
        {
            type Output = T;
            async fn get(self, _: LazyContext<'_>) -> LazyOutput<E, Self::Output> {
                tokio::task::spawn_blocking(self.0)
                    .map(|res| res.expect("task shouldn't panic"))
                    .map(LazyOutput::pure)
                    .await
            }
        }
        Blocking(func)
    }

    pub fn future(fut: impl Future<Output = T> + Send + 'static) -> impl Lazy<E, Output = T> {
        struct LazyFuture<F>(F);
        impl<T, F, E> Lazy<E> for LazyFuture<F>
        where
            T: Send + 'static,
            F: Future<Output = T> + Send + 'static,
            E: Effect,
        {
            type Output = T;
            fn get(self, _: LazyContext<'_>) -> impl Future<Output = LazyOutput<E, T>> + Send + '_ {
                self.0.map(LazyOutput::pure)
            }
        }
        LazyFuture(fut)
    }

    pub fn todo() -> impl Lazy<E, Output = T> {
        Self::future(async { todo!() })
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
                    return LazyOutput::err(ModuleError::NotFound { id: M::id() });
                };

                let module = module.read().await;
                let value = (self.fut)(&module).await;
                drop(module);

                LazyOutput::pure(value)
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
                    return LazyOutput::err(ModuleError::NotFound { id: M::id() });
                };

                let mut module = module.write().await;
                let value = (self.fut)(&mut module).await;
                drop(module);

                LazyOutput::pure(value)
            }
        }

        With {
            fut,
            pd: PhantomData,
        }
    }

    pub fn looping<F, L, C>(initial: C, func: F) -> impl Lazy<E, Output = T>
    where
        F: FnMut(C) -> L + Send + 'static,
        L: Lazy<E, Output = ControlFlow<T, C>>,
        C: Send + 'static,
    {
        struct Looping<F, C> {
            func: F,
            value: C,
        }

        impl<E, F, L, C, T> Lazy<E> for Looping<F, C>
        where
            E: Effect,
            F: FnMut(C) -> L + Send + 'static,
            L: Lazy<E, Output = ControlFlow<T, C>>,
            C: Send + 'static,
            T: Send + 'static,
        {
            type Output = T;

            async fn get(self, context: LazyContext<'_>) -> LazyOutput<E, Self::Output> {
                let Self {
                    mut func,
                    mut value,
                } = self;

                let result = loop {
                    let flow = run_lazy_inner!(func(value), context);
                    match flow {
                        ControlFlow::Continue(new_value) => value = new_value,
                        ControlFlow::Break(result) => break result,
                    }
                };

                LazyOutput::pure(result)
            }
        }

        Looping {
            func,
            value: initial,
        }
    }

    pub fn breaking<F, L>(func: F) -> impl Lazy<E, Output = T>
    where
        E: MakeBreak<T>,
        F: FnOnce(fn(T) -> Break<T, E>) -> L + Send + 'static,
        L: Lazy<<E as MakeBreak<T>>::Make, Output = T>,
    {
        struct Breaking<F, PD> {
            func: F,
            pd: PhantomData<PD>,
        }

        impl<E, F, L, T> Lazy<E> for Breaking<F, (L, T)>
        where
            E: MakeBreak<T>,
            F: FnOnce(fn(T) -> Break<T, E>) -> L + Send + 'static,
            L: Lazy<<E as MakeBreak<T>>::Make, Output = T>,
            T: Send + 'static,
        {
            type Output = T;

            async fn get(self, context: LazyContext<'_>) -> LazyOutput<E, Self::Output> {
                let lazy = (self.func)(Break::new);
                let output = lazy.get(context).await;

                match output.value {
                    Ok(value) => {
                        match <<E as MakeBreak<T>>::Make as Effect>::Breakability::catch(value) {
                            ControlFlow::Continue(value) => LazyOutput::breaking(value),
                            ControlFlow::Break(value) => LazyOutput::pure(value),
                        }
                    }
                    Err(err) => LazyOutput::err(err),
                }
            }
        }

        Breaking {
            func,
            pd: PhantomData,
        }
    }
}

pub struct Break<T, E> {
    value: T,
    pd: PhantomData<E>,
}

impl<T, E> Break<T, E> {
    const fn new(value: T) -> Self {
        Self {
            value,
            pd: PhantomData,
        }
    }
}

impl<T, E, E2> Lazy<E2> for Break<T, E>
where
    T: Send + 'static,
    E: Effect + MakePartial + Apply<<Breakable<T> as Partial>::Use>,
    (E::Partial, Breakable<T>): Effect<Breakability: BreakOut<Outer = T>>,
    E2: Contains<(E::Partial, Breakable<T>)>,
{
    type Output = !;

    async fn get(self, _: LazyContext<'_>) -> LazyOutput<E2, Self::Output> {
        LazyOutput::<(E::Partial, Breakable<T>), _>::break_out(self.value).with_effect()
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
                    return LazyOutput::pure(None);
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
    type Error: From<!> + Send + 'static;

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
                        return LazyOutput::pure(Err(err));
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
        E: Contains<(Pure, Fallible<Self::Error>)>,
    {
        struct CancelOnError<L>(L);

        impl<L, E, T, Err> Lazy<E> for CancelOnError<L>
        where
            L: Lazy<E, Output = Result<T, Err>>,
            T: Send + 'static,
            E: Contains<(Pure, Fallible<Err>)>,
            Err: From<!> + Send + 'static,
        {
            type Output = T;
            async fn get(self, context: LazyContext<'_>) -> LazyOutput<E, Self::Output> {
                match run_lazy_inner!(self.0, context) {
                    Ok(value) => LazyOutput::pure(value),
                    Err(err) => LazyOutput::err(err),
                }
            }
        }

        CancelOnError(self)
    }
}

impl<T, Err, E, L> LazyResult<E> for L
where
    T: Send + 'static,
    Err: From<!> + Send + 'static,
    E: Effect,
    L: Lazy<E, Output = Result<T, Err>>,
{
    type Inner = T;
    type Error = Err;
}

pub trait LazyRecursive<E: Effect>: Lazy<E, Output = Self::InnerLazy> {
    type InnerValue: Send + 'static;
    type InnerLazy: Lazy<E, Output = Self::InnerValue>;

    fn flatten(self) -> impl Lazy<E, Output = Self::InnerValue> {
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

pub trait LazyFallible<E: Effect<Fallibility = Infallible>, Err: From<!> + Send + 'static>:
    Lazy<(E::Exclusivity, E::Breakability, Fallible<Err>)>
where
    (E::Exclusivity, E::Breakability, Fallible<Err>):
        Effect<Breakability = E::Breakability, Fallibility = Fallible<Err>>,
{
    fn catch(self) -> impl Lazy<E, Output = Result<Self::Output, Err>> {
        struct Catch<L, E> {
            lazy: L,
            pd: PhantomData<E>,
        }

        impl<E, Err, L> Lazy<E> for Catch<L, Err>
        where
            E: Effect<Fallibility = Infallible>,
            Err: From<!> + Send + 'static,
            (E::Exclusivity, E::Breakability, Fallible<Err>):
                Effect<Breakability = E::Breakability, Fallibility = Fallible<Err>>,
            L: Lazy<(E::Exclusivity, E::Breakability, Fallible<Err>)>,
        {
            type Output = Result<L::Output, Err>;
            async fn get(self, context: LazyContext<'_>) -> LazyOutput<E, Self::Output> {
                let value = self.lazy.get(context).await;
                match value.extract().map_err(|err| err.value) {
                    Ok(value) => LazyOutput::pure(Ok(value)),
                    Err(Ok(breaking)) => LazyOutput::breaking(breaking),
                    Err(Err(error)) => LazyOutput::pure(Err(error)),
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
    E: Effect<Fallibility = Infallible>,
    Err: From<!> + Send + 'static,
    L: Lazy<(E::Exclusivity, E::Breakability, Fallible<Err>)>,
    (E::Exclusivity, E::Breakability, Fallible<Err>):
        Effect<Breakability = E::Breakability, Fallibility = Fallible<Err>>,
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
    E: Effect,
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

impl From<!> for ModuleError {
    fn from(value: !) -> Self {
        value
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        effect::{self, Effect, Fallible, Pure},
        global::GlobalState,
        lazy::{Laze, Lazy, LazyContext, LazyFallible, LazyResult},
    };

    async fn run<E: Effect, L: Lazy<E, Output = T>, T: Send + 'static>(
        lazy: L,
    ) -> effect::CleanOutput<T, E> {
        let state = GlobalState::default();
        let context = LazyContext { state: &state };

        let output = lazy.get(context).await;
        effect::clean::<T, E>(output.value)
    }

    macro_rules! assert_lazy_eq {
        ($left:expr, $right:expr) => {
            assert_lazy_eq!((Pure,) => ($left, $right))
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
        assert_lazy_eq!(Laze::blocking(|| 5), 5).await;
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

    #[derive(PartialEq, Eq, Debug)]
    struct Empty;
    impl From<!> for Empty {
        fn from(value: !) -> Self {
            value
        }
    }

    #[tokio::test]
    async fn throw() {
        assert_lazy_eq!((Fallible::<Empty>,) => (
            Laze::just(Err(Empty)).throw(),
            Result::<(), Empty>::Err(Empty)
        ))
        .await;
    }

    #[tokio::test]
    async fn then_fallible() {
        assert_lazy_eq!((Fallible::<Empty>,) => (
            Laze::just(()).then(|()| Laze::just(Err(Empty)).throw()),
            Result::<(), Empty>::Err(Empty)
        ))
        .await;
    }

    #[tokio::test]
    async fn throw_catch() {
        assert_lazy_eq!(
            Laze::just(Err(Empty)).throw().catch(),
            Result::<(), Empty>::Err(Empty)
        )
        .await;
    }

    #[tokio::test]
    async fn boxed() {
        assert_lazy_eq!(Laze::just(5).boxed(), 5).await;
    }

    #[tokio::test]
    async fn immediate_break() {
        assert_lazy_eq!(Laze::breaking(|break_with| break_with(5).map(|n| n)), 5).await;
    }
}
