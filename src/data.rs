use std::collections::HashMap;
use std::hash::Hash;
use std::path::PathBuf;
use std::time::Duration;

use auditeur::namespaced_id;
use auditeur::namespaced_id::{NamespacedIdRef, ident};
use facet::Facet;

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
