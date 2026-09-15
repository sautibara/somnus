use std::marker::PhantomData;

use crate::{
    effect::{Effect, Partial},
    partial,
};

pub trait Fallibility: Partial<Use = Kind<Self>> + Sized + Send + Sync + 'static {
    type Error: From<!> + Send + 'static;

    type CleanOutput<T: Send + 'static>: Send + 'static;
    fn clean_output<T: Send + 'static>(value: Result<T, Self::Error>) -> Self::CleanOutput<T>;
}

pub struct Infallible(PhantomData<()>);
pub struct Fallible<E>(PhantomData<fn() -> E>);

partial!(Infallible, Kind<Self>);
partial!(Fallible<E>, Kind<Self>, (<E: From<!> + Send + 'static>));

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

pub struct Kind<F: Fallibility>(PhantomData<F>);
impl<F: Fallibility> Effect for Kind<F> {
    type Exclusivity = <() as Effect>::Exclusivity;
    type Fallibility = F;
    type Breakability = <() as Effect>::Breakability;
}

pub trait Contains<F: Fallibility>: Fallibility<Error: From<F::Error>> {}
impl<Outer, Inner> Contains<Inner> for Outer
where
    Inner: Fallibility,
    Outer: Fallibility,
    Outer::Error: From<Inner::Error>,
{
}

pub trait Apply<Other: Fallibility>: Fallibility {
    type And: Fallibility + Contains<Self>;
}

impl<F: Fallibility> Apply<F> for Infallible {
    type And = F;
}

impl<F: Fallibility, E: From<!> + Send + 'static> Apply<F> for Fallible<E> {
    type And = Self;
}
