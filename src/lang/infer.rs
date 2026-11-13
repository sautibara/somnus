#![allow(clippy::missing_const_for_fn)]
#![allow(dead_code)]

// NOTE: This type inference scheme is *heavily* inspired by Thunderseethe's making a language
// series (https://thunderseethe.dev/series/making-a-language/)

use std::collections::HashMap;

use ena::unify::{EqUnifyValue, InPlaceUnificationTable, UnifyKey};
use thiserror::Error;

use crate::lang::ast::NodeId;

use super::ast;

#[derive(Debug)]
pub struct InferOut {
    pub expr: ast::Expr<TypedVar>,
    pub ty: Type,
    pub errors: HashMap<NodeId, Error>,
}

pub fn type_infer(expr: ast::Expr<Var>) -> InferOut {
    infer(move |ctx| ctx.infer(imbl::HashMap::default(), expr))
}

pub fn type_check(expr: ast::Expr<Var>, ty: Type) -> InferOut {
    infer(move |ctx| (ty.clone(), ctx.check(imbl::HashMap::default(), expr, ty)))
}

fn infer(call: impl FnOnce(&mut TypeInference) -> (Type, GenOut)) -> InferOut {
    let mut ctx = TypeInference::default();

    let (ty, out) = call(&mut ctx);

    let mut errors = ctx.unify(out.constraints);

    let (expr, subsitute_errors) = ctx.substitute_ast(out.typed_ast);
    errors.extend(subsitute_errors);
    let (ty, has_unbound) = ctx.normalize_ty(ty);

    if has_unbound {
        errors.insert(expr.id(), ErrorKind::FreeTypeVariable.with_id(expr.id()));
    }

    InferOut { expr, ty, errors }
}

/// A type variable, for use in the unification table.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct TypeVar(u32);

/// The type of an expression.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Type {
    Var(TypeVar),
    Prim(Primitive),
    Const(ast::Primitive),
    Fun { arg: Box<Self>, ret: Box<Self> },
}

impl Type {
    fn fun(arg: Self, ret: Self) -> Self {
        Self::Fun {
            arg: Box::new(arg),
            ret: Box::new(ret),
        }
    }
}

/// The type of an arbitrary primitive.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Primitive {
    None,
    Boolean,
    Number,
    String,
    Uuid,
}

impl Primitive {
    fn of(prim: &ast::Primitive) -> Self {
        match prim {
            ast::Primitive::None => Self::None,
            ast::Primitive::Boolean(_) => Self::Boolean,
            ast::Primitive::Number(_) => Self::Number,
            ast::Primitive::String(_) => Self::String,
            ast::Primitive::Uuid(_) => Self::Uuid,
        }
    }
}

/// A variable with an uninferred type.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub struct Var(usize);
/// A variable with an inferred type.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct TypedVar(Var, Type);

/// State used to infer types in an ast.
#[derive(Default)]
pub struct TypeInference {
    unification_table: InPlaceUnificationTable<TypeVar>,
}

/// A constraint produced by type inference.
///
/// These constraints will be used to complete inference.
enum Constraint {
    TypeEqual(NodeId, Type, Type),
}

/// The result of an infer pass.
struct GenOut {
    /// A set of constraints to be solved.
    constraints: Vec<Constraint>,
    /// The output ast, where all variables are annotated with a type.
    /// This type may be an uninferred type variable, though.
    typed_ast: ast::Expr<TypedVar>,
}

impl TypeInference {
    fn fresh_ty_var(&mut self) -> TypeVar {
        self.unification_table.new_key(None)
    }

    /// Infers the type of `ast`, returning the type and any constraints necessary to make sure
    /// the type matches (the type will likely include type variables that need to be unified).
    fn infer(&mut self, env: imbl::HashMap<Var, Type>, ast: ast::Expr<Var>) -> (Type, GenOut) {
        match ast {
            ast::Expr::Primitive { id, value } => (
                Type::Const(value.clone()),
                GenOut::new(ast::Expr::Primitive { id, value }),
            ),
            ast::Expr::Var { id, value } => {
                let ty = env
                    .get(&value)
                    .expect("ICE: name resolution should ensure variables are defined before use");
                (
                    ty.clone(),
                    GenOut::new(ast::Expr::Var {
                        id,
                        value: TypedVar(value, ty.clone()),
                    }),
                )
            }
            ast::Expr::Hole { id, value } => {
                // Holes can take on any type, so just make it a new type variable.
                let ty = Type::Var(self.fresh_ty_var());
                (
                    ty.clone(),
                    GenOut::new(ast::Expr::Hole {
                        id,
                        value: TypedVar(value, ty),
                    }),
                )
            }
            ast::Expr::Fun { id, param, body } => {
                // We have a new variable, so we have to make a new type variable.
                let arg_ty = Type::Var(self.fresh_ty_var());
                // And add our argument type temporarily to the environment for this call.
                let env = env.update(param, arg_ty.clone());

                let (body_ty, body_out) = self.infer(env, *body);
                (
                    Type::fun(arg_ty.clone(), body_ty),
                    body_out.wrap(|body| ast::Expr::fun(id, TypedVar(param, arg_ty.clone()), body)),
                )
            }
            ast::Expr::Call { id, fun, arg } => {
                // Our argument type is inferred recursively,
                let (arg_ty, arg_out) = self.infer(env.clone(), *arg);
                // and our return type will depend on the function's body and that argument,
                // so it should be a new free type variable.
                let ret_ty = Type::Var(self.fresh_ty_var());
                // Then, now that we have those two types, we could check the function type.
                let fun_out = self.check(env, *fun, Type::fun(arg_ty, ret_ty.clone()));

                (
                    ret_ty,
                    fun_out.zip(arg_out, |fun, arg| ast::Expr::call(id, fun, arg)),
                )
            }
            ast::Expr::AssertTy { id, body, ty } => {
                let out = self.check(env, *body, ty.clone());
                (
                    ty.clone(),
                    out.wrap(|body| ast::Expr::assert_ty(id, body, ty)),
                )
            }
        }
    }

    /// Returns the constraints necessary to ensure that the type of `ast` matches `ty`.
    fn check(&mut self, env: imbl::HashMap<Var, Type>, ast: ast::Expr<Var>, ty: Type) -> GenOut {
        match (ast, ty) {
            // For primitives, we could directly check the type.
            (ast::Expr::Primitive { id, value }, Type::Prim(ty)) if Primitive::of(&value) == ty => {
                GenOut::new(ast::Expr::Primitive { id, value })
            }
            // For functions, we could directly check that the body matches the return type.
            (ast::Expr::Fun { id, param, body }, Type::Fun { arg, ret }) => {
                let env = env.update(param, *arg.clone());
                let body_out = self.check(env, *body, *ret);
                body_out.wrap(|body| ast::Expr::fun(id, TypedVar(param, *arg), body))
            }
            // Otherwise, we add a type equality constraint.
            (ast, expected_ty) => {
                let node_id = ast.id();
                let (actual_ty, out) = self.infer(env, ast);
                out.with([Constraint::TypeEqual(node_id, expected_ty, actual_ty)])
            }
        }
    }

    /// Attempts to find a valid type scheme such that all `constraints` succeed.
    ///
    /// Errors are reported per-node in the returned map.
    fn unify(&mut self, constraints: Vec<Constraint>) -> HashMap<NodeId, Error> {
        let mut errors = HashMap::new();
        for constraint in constraints {
            match constraint {
                Constraint::TypeEqual(id, left, right) => {
                    if let Err(kind) = self.unify_ty_ty(left, right) {
                        errors.insert(id, Error { node: id, kind });
                    }
                }
            }
        }
        errors
    }

    /// Attempts to find a valid type scheme such that the `left` and `right` types are equal.
    fn unify_ty_ty(
        &mut self,
        left_unnormalized: Type,
        right_unnormalized: Type,
    ) -> Result<(), ErrorKind> {
        let (left, _) = self.normalize_ty(left_unnormalized);
        let (right, _) = self.normalize_ty(right_unnormalized);
        match (left, right) {
            (Type::Prim(left), Type::Prim(right)) if left == right => Ok(()),
            (Type::Const(left), Type::Const(right)) if left == right => Ok(()),
            (
                Type::Fun {
                    arg: left_arg,
                    ret: left_ret,
                },
                Type::Fun {
                    arg: right_arg,
                    ret: right_ret,
                },
            ) => {
                self.unify_ty_ty(*left_arg, *right_arg)?;
                self.unify_ty_ty(*left_ret, *right_ret)?;
                Ok(())
            }
            (Type::Var(left), Type::Var(right)) => self
                .unification_table
                .unify_var_var(left, right)
                .map_err(|(left, right)| ErrorKind::TypeNotEqual(left, right)),
            (Type::Var(v), ty) | (ty, Type::Var(v)) => {
                // TODO: occurs check
                self.unification_table
                    .unify_var_value(v, Some(ty))
                    .map_err(|(left, right)| ErrorKind::TypeNotEqual(left, right))
            }
            (left, right) => Err(ErrorKind::TypeNotEqual(left, right)),
        }
    }

    /// Applies all already-solved type subsitutions to the type variables in `ty`,
    /// returning the resulting type and `true` iff there is any free type variable in it.
    fn normalize_ty(&mut self, ty: Type) -> (Type, bool) {
        match ty {
            Type::Var(type_var) => match self.unification_table.probe_value(type_var) {
                Some(ty) => self.normalize_ty(ty),
                None => (Type::Var(self.unification_table.find(type_var)), true),
            },
            Type::Prim(primitive) => (Type::Prim(primitive), false),
            Type::Const(primitive) => (Type::Const(primitive), false),
            Type::Fun { arg, ret } => {
                let (arg, arg_has_free) = self.normalize_ty(*arg);
                let (ret, ret_has_free) = self.normalize_ty(*ret);
                (Type::fun(arg, ret), arg_has_free || ret_has_free)
            }
        }
    }

    fn substitute_ast(
        &mut self,
        ast: ast::Expr<TypedVar>,
    ) -> (ast::Expr<TypedVar>, HashMap<NodeId, Error>) {
        match ast {
            // Var and Hole are pretty much identical.
            ast::Expr::Var {
                id,
                value: TypedVar(var, ty),
            } => {
                // NOTE: We only report free type errors at the variable definition,
                // so ignore any free type variables here.
                let (ty, _) = self.normalize_ty(ty);
                (ast::Expr::var(id, TypedVar(var, ty)), HashMap::new())
            }
            ast::Expr::Hole {
                id,
                value: TypedVar(var, ty),
            } => {
                let (ty, _) = self.normalize_ty(ty);
                (ast::Expr::hole(id, TypedVar(var, ty)), HashMap::new())
            }
            ast::Expr::Fun {
                id,
                param: TypedVar(var, ty),
                body,
            } => {
                let (body, mut errors) = self.substitute_ast(*body);

                let (ty, has_free) = self.normalize_ty(ty);
                if has_free {
                    errors.insert(id, ErrorKind::FreeTypeVariable.with_id(id));
                }

                (ast::Expr::fun(id, TypedVar(var, ty), body), errors)
            }
            ast::Expr::Call { id, fun, arg } => {
                let (fun, mut errors) = self.substitute_ast(*fun);
                let (arg, arg_errors) = self.substitute_ast(*arg);
                errors.extend(arg_errors);
                (ast::Expr::call(id, fun, arg), errors)
            }
            ast::Expr::AssertTy { id, body, ty } => {
                let (body, errors) = self.substitute_ast(*body);
                // NOTE: Theoretically `ty` shouldn't ever have to be normalized (since we're
                // asserting the type - it shouldn't have any type variables), but we still do in
                // case some other pass introduces a weird assert.
                let (ty, _) = self.normalize_ty(ty);
                (ast::Expr::assert_ty(id, body, ty), errors)
            }
            ast::Expr::Primitive { id, value } => (ast::Expr::primitive(id, value), HashMap::new()),
        }
    }
}

impl GenOut {
    fn new(ast: ast::Expr<TypedVar>) -> Self {
        Self {
            constraints: vec![],
            typed_ast: ast,
        }
    }

    fn wrap(self, with: impl FnOnce(ast::Expr<TypedVar>) -> ast::Expr<TypedVar>) -> Self {
        Self {
            constraints: self.constraints,
            typed_ast: with(self.typed_ast),
        }
    }

    fn zip(
        self,
        other: Self,
        with: impl FnOnce(ast::Expr<TypedVar>, ast::Expr<TypedVar>) -> ast::Expr<TypedVar>,
    ) -> Self {
        Self {
            constraints: self
                .constraints
                .into_iter()
                .chain(other.constraints)
                .collect(),
            typed_ast: with(self.typed_ast, other.typed_ast),
        }
    }

    fn with<T: IntoIterator<Item = Constraint>>(self, constraints: T) -> Self {
        Self {
            constraints: self.constraints.into_iter().chain(constraints).collect(),
            typed_ast: self.typed_ast,
        }
    }
}

impl UnifyKey for TypeVar {
    type Value = Option<Type>;

    fn index(&self) -> u32 {
        self.0
    }

    fn from_index(u: u32) -> Self {
        Self(u)
    }

    fn tag() -> &'static str {
        "TypeVar"
    }
}

impl EqUnifyValue for Type {}

#[derive(Debug, Clone, Error)]
#[error("{kind}")]
pub struct Error {
    node: ast::NodeId,
    kind: ErrorKind,
}

#[derive(Debug, Clone, Error)]
pub enum ErrorKind {
    #[error("types not equal: {0:?} and {1:?}")]
    TypeNotEqual(Type, Type),
    #[error("type could not be determined")]
    FreeTypeVariable,
}

impl ErrorKind {
    fn with_id(self, node: ast::NodeId) -> Error {
        Error { node, kind: self }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use insta::assert_debug_snapshot;

    use crate::lang::{
        ast::{Expr, NodeId, Primitive},
        infer::{self, Type, Var, type_check, type_infer},
    };

    #[derive(Default)]
    struct Builder {
        next_node_id: Cell<usize>,
        next_var_id: Cell<usize>,
    }

    impl Builder {
        fn node_id(&self) -> NodeId {
            let id = NodeId(self.next_node_id.get());
            self.next_node_id.update(|v| v + 1);
            id
        }

        fn var_id(&self) -> Var {
            let id = Var(self.next_var_id.get());
            self.next_var_id.update(|v| v + 1);
            id
        }

        fn var(&self, var: Var) -> Expr<Var> {
            Expr::var(self.node_id(), var)
        }

        fn fun(&self, param: Var, body: Expr<Var>) -> Expr<Var> {
            Expr::fun(self.node_id(), param, body)
        }

        fn call(&self, func: Expr<Var>, arg: Expr<Var>) -> Expr<Var> {
            Expr::call(self.node_id(), func, arg)
        }

        fn assert_ty(&self, ty: infer::Type, body: Expr<Var>) -> Expr<Var> {
            Expr::assert_ty(self.node_id(), body, ty)
        }

        fn prim(&self, primitive: Primitive) -> Expr<Var> {
            Expr::primitive(self.node_id(), primitive)
        }

        fn hole(&self) -> Expr<Var> {
            Expr::hole(self.node_id(), self.var_id())
        }
    }

    #[test]
    fn infers_int() {
        let b = Builder::default();
        let expr = b.prim(Primitive::number(1));

        assert_debug_snapshot!(type_infer(expr));
    }

    #[test]
    fn infers_int_identity_fn() {
        let b = Builder::default();
        let var = b.var_id();
        let expr = b.call(b.fun(var, b.var(var)), b.prim(Primitive::number(1)));

        assert_debug_snapshot!(type_infer(expr));
    }

    #[test]
    fn rejects_unbound_identity_fn() {
        let b = Builder::default();
        let var = b.var_id();
        let expr = b.fun(var, b.var(var));

        assert_debug_snapshot!(type_infer(expr));
    }

    #[test]
    fn infers_assert_ty_identity_fn() {
        let b = Builder::default();
        let var = b.var_id();
        let identity = b.fun(var, b.var(var));

        let ty = Type::fun(
            Type::Prim(infer::Primitive::Number),
            Type::Prim(infer::Primitive::Number),
        );
        let expr = b.assert_ty(ty, identity);

        assert_debug_snapshot!(type_infer(expr));
    }

    #[test]
    fn checks_identity_fn() {
        let b = Builder::default();
        let var = b.var_id();
        let expr = b.fun(var, b.var(var));

        let ty = Type::fun(
            Type::Prim(infer::Primitive::Number),
            Type::Prim(infer::Primitive::Number),
        );

        assert_debug_snapshot!(type_check(expr, ty));
    }

    #[test]
    fn rejects_bad_assert_ty_identity_fn() {
        let b = Builder::default();
        let var = b.var_id();
        let identity = b.fun(var, b.var(var));

        let ty = Type::fun(
            Type::Prim(infer::Primitive::String),
            Type::Prim(infer::Primitive::Number),
        );
        let expr = b.assert_ty(ty, identity);

        assert_debug_snapshot!(type_infer(expr));
    }

    #[test]
    fn accepts_hole_assert_ty() {
        let b = Builder::default();

        let hole = b.hole();
        let expr = b.assert_ty(Type::Prim(infer::Primitive::Number), hole);

        assert_debug_snapshot!(type_infer(expr));
    }
}
