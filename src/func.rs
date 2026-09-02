use crate::{
    effect::{Contains, Effect},
    lazy::{BoxLazy, Lazy},
};

pub trait LazyFn<E: Effect, Args: Send + 'static>: Sized + Send + 'static {
    type Output: Send + 'static;

    fn call_inner(&self, args: Args) -> impl Lazy<E, Output = Self::Output>;

    fn awaken<E2>(&self, args: Args) -> impl Lazy<E2, Output = Self::Output>
    where
        E2: Effect + Contains<E>,
    {
        self.call_inner(args).using_effect()
    }

    fn boxed(self) -> BoxLazyFn<E, Args, Self::Output> {
        BoxLazyFn {
            func: Box::new(move |args| self.call_inner(args).boxed()),
        }
    }
}

pub struct BoxLazyFn<E: Effect, Args: Send + 'static, O: Send + 'static> {
    func: Box<dyn Fn(Args) -> BoxLazy<E, O> + Send + 'static>,
}

impl<E: Effect, Args: Send + 'static, O: Send + 'static> LazyFn<E, Args> for BoxLazyFn<E, Args, O> {
    type Output = O;

    fn call_inner(&self, args: Args) -> impl Lazy<E, Output = Self::Output> {
        (self.func)(args)
    }

    fn boxed(self) -> BoxLazyFn<E, Args, Self::Output> {
        self
    }
}

macro_rules! impl_lazy_fn {
    ($(($T:ident, $t:ident)),*) => {
        impl<E, O, L, $($T,)* F> LazyFn<E, ($($T,)*)> for F
        where
            E: Effect,
            O: Send + 'static,
            L: Lazy<E, Output = O>,
            F: Fn($($T),*) -> L + Send + 'static,
            $($T: Send + 'static,)*
        {
            type Output = O;
            fn call_inner(&self, ($($t,)*): ($($T,)*)) -> impl Lazy<E, Output = O> {
                self($($t),*)
            }
        }
    };
}

bevy_utils_proc_macros::all_tuples!(impl_lazy_fn, 0, 8, T, t);
