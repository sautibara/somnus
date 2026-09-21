use std::collections::HashMap;
use std::hash::Hash;
use std::path::PathBuf;
use std::time::Duration;

use auditeur::namespaced_id;
use auditeur::namespaced_id::{NamespacedIdRef, ident};
use facet::Facet;
use thiserror::Error;

// NOTE: I'm hoping that [`Data`] won't have to be added as a component.
// Even the different [`Data`] implementations for the standard library are already a lot.
// This will probably be fine as long as we always store undo/redo actions as Facet values.

pub trait Data: Facet<'static> + Send + Sync + 'static {
    fn id() -> &'static NamespacedIdRef;
}

macro_rules! impl_data {
    (
        $(
            $ty:ty
            $(where ($(
                $generic:ident
                $(: $first_bound:ident $(+ $bound:ident)*)?
            ),*))?
            : $ident:literal
        ),* ,
    ) => {
        $(
            impl $(<
                $($generic: Facet<'static> + Send + Sync $(+ $first_bound $(+ $bound)*)?),*
            >)? Data for $ty {
                fn id() -> &'static NamespacedIdRef {
                    ident!($ident)
                }
            }
        )*
    };
}

impl_data!(
    (): "std:tuple",
    (T0,) where (T0): "std:tuple",
    (T0, T1) where (T0, T1): "std:tuple",
    (T0, T1, T2) where (T0, T1, T2): "std:tuple",
    (T0, T1, T2, T3) where (T0, T1, T2, T3): "std:tuple",

    u8: "std:uint",
    u16: "std:uint",
    u32: "std:uint",
    u64: "std:uint",
    u128: "std:uint",
    usize: "std:uint",

    i8: "std:sint",
    i16: "std:sint",
    i32: "std:sint",
    i64: "std:sint",
    i128: "std:sint",
    isize: "std:sint",

    f32: "std:float",
    f64: "std:float",

    bool: "std:bool",
    char: "std:char",

    String: "std:string",
    Duration: "std:duration",
    PathBuf: "std:path_buf",

    Vec<T> where (T): "std:vec",
    HashMap<K, V> where (K: Eq + Hash, V): "std:map",

    Option<T> where (T): "std:option",
    Result<T, E> where (T, E): "std:result",

    Box<[T]> where (T): "std:boxed_slice",
    Box<str>: "std:boxed_str",
);

pub struct Value {
    facet: facet_value::Value,
}

impl Value {
    /// # Errors
    ///
    /// - If the value could not be represented.
    #[allow(clippy::needless_pass_by_value)]
    pub fn try_from<D: Data>(data: D) -> Result<Self, FromError> {
        Ok(Self {
            facet: facet_value::to_value(&data)?,
        })
    }

    /// # Errors
    ///
    /// - If `D` cannot be deserialized from the contained value.
    pub fn try_into<D: Data>(self) -> Result<D, IntoError> {
        facet_value::from_value(self.facet).map_err(Into::into)
    }
}

#[derive(Error, Debug)]
pub enum FromError {
    #[error("{0}")]
    Facet(#[from] facet_format::SerializeError<facet_value::ToValueError>),
}

#[derive(Error, Debug)]
pub enum IntoError {
    #[error("{0}")]
    Facet(#[source] Box<facet_value::ValueError>),
}

impl From<facet_value::ValueError> for IntoError {
    fn from(value: facet_value::ValueError) -> Self {
        Self::Facet(Box::new(value))
    }
}

#[derive(Error, Debug)]
pub enum Error {
    #[error("failed to serialize some data into a type-erased value: {0}")]
    From(#[from] FromError),
    #[error("failed to deserialize a value into some typed data: {0}")]
    Into(#[from] IntoError),
}
