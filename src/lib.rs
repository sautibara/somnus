#![doc = include_str!("../README.md")]
#![feature(unboxed_closures, async_fn_traits)]

// LazyFn is the source of all undo - for an action to be undoable you must call a LazyFn
// This is so we could have the arguments necessary to redo

pub mod data;
pub mod effect;
pub mod error;
pub mod func;
pub mod global;
pub mod lazy;
pub mod module;

pub use error::Error;
