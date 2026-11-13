#![allow(dead_code)]
#![allow(clippy::missing_const_for_fn)]

use std::{fmt::Debug, hash::Hash};

use num_order::NumHash;

use crate::lang::infer;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct NodeId(pub usize);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Expr<V> {
    /// Local variable access.
    Var { id: NodeId, value: V },
    /// Function definition.
    Fun {
        id: NodeId,
        param: V,
        body: Box<Self>,
    },
    /// Function call.
    Call {
        id: NodeId,
        fun: Box<Self>,
        arg: Box<Self>,
    },
    /// Ensures that `body` has the type `ty`.
    AssertTy {
        id: NodeId,
        body: Box<Self>,
        ty: infer::Type,
    },
    /// Primitives.
    Primitive { id: NodeId, value: Primitive },
    /// An error, whose type is similar to a never type.
    Hole { id: NodeId, value: V },
}

impl<V> Expr<V> {
    pub fn id(&self) -> NodeId {
        match self {
            Self::Var { id, .. }
            | Self::Fun { id, .. }
            | Self::Call { id, .. }
            | Self::AssertTy { id, .. }
            | Self::Primitive { id, .. }
            | Self::Hole { id, .. } => *id,
        }
    }

    pub fn var(id: NodeId, value: V) -> Self {
        Self::Var { id, value }
    }

    pub fn fun(id: NodeId, param: V, body: Self) -> Self {
        Self::Fun {
            id,
            param,
            body: Box::new(body),
        }
    }

    pub fn call(id: NodeId, func: Self, arg: Self) -> Self {
        Self::Call {
            id,
            fun: Box::new(func),
            arg: Box::new(arg),
        }
    }

    pub fn assert_ty(id: NodeId, body: Self, ty: infer::Type) -> Self {
        Self::AssertTy {
            id,
            body: Box::new(body),
            ty,
        }
    }

    pub fn primitive(id: NodeId, value: Primitive) -> Self {
        Self::Primitive { id, value }
    }

    pub fn hole(id: NodeId, value: V) -> Self {
        Self::Hole { id, value }
    }
}

#[derive(Clone, PartialEq, Eq)]
pub struct Number {
    value: dashu_float::FBig,
}

impl Number {
    fn new(value: dashu_float::FBig) -> Self {
        Self { value }
    }
}

impl Hash for Number {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.value.num_hash(state);
    }
}

impl From<usize> for Number {
    fn from(value: usize) -> Self {
        Self::new(dashu_float::FBig::from(value))
    }
}

impl Debug for Number {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.value.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Primitive {
    None,
    Boolean(bool),
    Number(Number),
    String(String),
    Uuid(uuid::Uuid),
}

impl Primitive {
    pub fn number(number: impl Into<Number>) -> Self {
        Self::Number(number.into())
    }
}
