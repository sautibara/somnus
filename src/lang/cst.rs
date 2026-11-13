#![allow(warnings)] // I'll come back to this later.

use facet::Facet;

// NOTE: no more currying

// Nonterminals with camel,
// Terminals surrounded in `` or screaming snake
//
// Expr ->
//     | Pipe
//
// Pipe ->
//     | Pipe `|` Unary
//     | Unary
//
// Unary ->
//     | Unary `.` IDENTIFIER
//     | Unary `(` ExprElements `)`
//     | `(` Expr `)`
//     | `<` PatElements `>` Expr
//     | Value
//
// Value ->
//     | ValueNoIdent
//     | IDENTIFIER
//
// ValueNoIdent ->
//     | Literal
//     | ExprObject
//     | ExprList
//
// Literal ->
//     | `none`
//     | `true | `false`
//     | NUMBER
//     | STRING
//     | UUID
//
// ExprObject ->
//     | `{` ExprPairs `}`
//
// ExprPairs ->
//     | ExprPair `,` ExprPairs
//     | <empty>
//
// ExprPair ->
//     | IDENTIFIER
//     | IDENTIFIER `:` Expr
//     | ValueNoIdent `:` Expr
//     | `(` Expr `)` `:` Expr
//     | `...` Unary
//
// ExprList ->
//     | `[` ExprElements `]`
//
// ExprElements ->
//     | ExprElement `,` ExprElements
//     | <empty>
//
// ExprElement ->
//     | Expr
//     | `...` Unary
//
// Patterns ->
//     | Pattern `,` Patterns
//     | <empty>
//
// Pattern ->
//     | IDENTIFIER
//     | PatObject
//     | PatList
//     | Literal
//
// PatObject ->
//     | `{` PatPairs `}`
//
// PatPairs ->
//     | PatPair `,` PatPairs
//     | <empty>
//
// PatPair ->
//     | IDENTIFIER
//     | IDENTIFIER `:` Pattern
//     | ValueNoIdent `:` Pattern
//     | `(` Expr `)` `:` Pattern
//     | `...` Pattern
//
// PatList ->
//     | `[` PatElements `]`
//
// PatElements ->
//     | PatElement `,` PatElements
//     | <empty>
//
// PatElement ->
//     | Pattern
//     | `...` Pattern

// TODO: add a type hint node for both expressions an patterns using : and on the same precedence level as |
// ex: a | b : c | d

// NOTE: model the type system based on thunderseethe's tutorial
// flow-sensitive typing is not necessary, since match uses functions

// TODO: Expr also encodes Values (no separate Value type)
// value serialization/deserialization goes in terms of Exprs
// canonical form of functions would be Expr -> Expr
// Expr -> value would be lazy and compute only what is necessary
// (which also allows for easy deserialization of generic Exprs)
//
// TODO: add a new node for value literals that will never be created in deserialization,
// but allows for lazy serialization and dynamic typing of values
// (would have methods for creating an Expr and Any)
//
// TODO: Exprs should only be calculated once - access will have to be lazy and mutate
// the variable to store already obtained data. This should work by converting lazy
// Exprs (function calls) to concrete exprs (array/object instantiation).
//
// Exprs could have a function to access a value through immutable reference, and if it's still
// arbitrary, automatically take a mutable reference to it and update it (assuming it's backed
// by a RwLock).

// TODO: use row types for type checking / inference
// rows specially have fields with *typed* keys (instead of always string keys like thunderseethe)
// objects, maps, tuples, and lists are all product types
// objects and maps use any type keys, potentially constants
// tuples and lists use only number keys, also potentially constants, maybe negative
// so the object { a: 5, b: "foo" } would have the type { "a": number, "b": string }
// a map could have the type { string: number }
// so we could assign the object { a: 5 } to that map, since the type string contains the type "a"
// a cursed map could have the type { "a": number, number: string }
// [0, ...list, 3] would use the row { 0: number, number: number, -1: number }
// honestly it might make sense to have lists be syntatic sugar for objects with number keys
// (oh no I'm making lua aren't I)
// union types have keys of the different types they could be
// so the type number | None would use the row { number: number, None: None }

#[derive(Facet)]
#[facet(proxy = SpanProxy)]
pub struct Span {
    range: std::ops::Range<usize>,
}

#[derive(Facet)]
#[facet(transparent)]
struct SpanProxy((usize, usize));

impl From<&Span> for SpanProxy {
    fn from(value: &Span) -> Self {
        Self((value.range.start, value.range.end))
    }
}

impl From<SpanProxy> for Span {
    fn from(SpanProxy((start, end)): SpanProxy) -> Self {
        Self { range: start..end }
    }
}

macro_rules! define_span_structs {
    ($($ident:ident),* $(,)?) => {
        $(
            #[derive(Facet)]
            #[facet(transparent)]
            pub struct $ident(Span);
        )*
    };
}

define_span_structs!(
    Paren, Angle, Square, Curly, Pipe, Comma, TripleDot, Dot, Dollar
);

#[macro_export]
macro_rules! Token {
    ( () ) => {
        $crate::lang::cst::Paren
    };
    ( <> ) => {
        $crate::lang::cst::Angle
    };
    ( [] ) => {
        $crate::lang::cst::Square
    };
    ( {} ) => {
        $crate::lang::cst::Curly
    };
    ( | ) => {
        $crate::lang::cst::Pipe
    };
    ( , ) => {
        $crate::lang::cst::Comma
    };
    ( ... ) => {
        $crate::lang::cst::TripleDot
    };
    ( . ) => {
        $crate::lang::cst::Dot
    };
    ( $ ) => {
        $crate::lang::cst::Dollar
    };
}

/// An expression in the `somnus` language.
#[derive(Facet)]
#[repr(u8)]
pub enum Expr {
    /// Method access, like `a.b(...)`.
    Dot {
        /// The value with the method, who will also be piped into the method.
        value: Box<Self>,
        /// The method name.
        identifier: Identifier,

        /// The parentheses spanning the arguments.
        paren: Token!(()),
        /// The arguments themselves.
        arguments: Vec<ExprListElement>,
    },
    /// A function call, like `a(...)`.
    Call {
        /// The expression being called.
        value: Box<Self>,

        /// The parentheses spanning the arguments.
        paren: Token!(()),
        /// The arguments themselves.
        arguments: Vec<ExprListElement>,
    },
    /// Expression grouping, like `(...)`.
    Group {
        /// The parentheses spanning the expression.
        paren: Token!(()),
        /// The expression contained.
        value: Box<Self>,
    },
    /// Value piping, like `a | b`.
    Pipe {
        /// The value to pipe into the right side function.
        from: Box<Self>,
        /// The span of the pipe token (for use in error messages).
        pipe: Token!(|),
        /// The function to pipe the left side value into.
        to: Box<Self>,
    },
    /// Variable access, like `a`.
    VariableAccess(Identifier),
    /// Value instantiation, like `0`, `[1, 2, 3]`, or `<a, b> a.sum(b)`.
    Value(Value),
}

/// An [`Expr`] that instantiates a value with a known type.
#[derive(Facet)]
#[repr(u8)]
pub enum Value {
    /// Object instantiation, like `{ a: b, ...c }`.
    Object(ExprObject),
    /// List instantiation, like `[a, b, ...c]`.
    List(ExprList),
    /// Literal instantiation, like `0`, `"a"`, `true`, or `None`.
    Literal(Literal),
    /// Function definition, like `<...> ...`.
    Function {
        /// The angle brackets spanning the parameters
        angle: Token!(<>),
        /// The parameters needed by the expression.
        elements: Vec<PatListElement>,
        /// The expression that's returned when called.
        value: Box<Self>,
    },
}

/// An [`Expr`] representing object instantiation, like `{ a: b, ...c }`.
#[derive(Facet)]
pub struct ExprObject {
    /// The curly brackets surrounding the object.
    pub curly: Token!({}),
    /// The parts - pairs or spread operations - that combine into the object.
    pub pairs: Vec<ExprObjectPair>,
}

/// A part of an [`ExprObject`], like `a: b` or `...a`.
#[derive(Facet)]
#[repr(u8)]
pub enum ExprObjectPair {
    /// A key-value pair, like `a: b`.
    KeyValue {
        /// The key of the pair, like `a`, `"a"`, or `(0)`.
        key: ExprObjectKey,
        /// The value of the pair.
        value: Box<Expr>,
    },
    /// A singular key, whose value will be accessed as a variable.
    Key(Identifier),
    /// A spread operation, like `...a`, `...a:b:c(...)`, or `...(...)`.
    Spread {
        /// The span of the spread token (for use in error messages).
        spread: Token!(...),
        /// The object value that should be spread.
        value: Box<Expr>,
    },
}

/// A key of an [`ExprObjectPair::KeyValue`], like `a`, `"a"`, or `(a)`.
#[derive(Facet)]
#[repr(u8)]
pub enum ExprObjectKey {
    /// An identifier that will be converted into a string key, like `a`.
    Identifier(Identifier),
    /// A literal that will be used as-is, like `0`.
    Literal(Literal),
    /// An expression whose output will be used as the key, like `(a)`.
    Expr { paren: Token!(()), value: Box<Expr> },
}

/// An [`Expr`] representing list instantiation, like `[a, b, ...c]`.
#[derive(Facet)]
pub struct ExprList {
    /// The square brackets surrounding the object.
    square: Token!([]),
    /// The parts - elements or spread operations - that combine into the list.
    elements: Vec<ExprListElement>,
}

/// An element of an [`ExprList`].
#[derive(Facet)]
#[repr(u8)]
pub enum ExprListElement {
    /// A singular value, like `a` or `0`.
    Single(Box<Expr>),
    /// A spread operation, like `...a`.
    Spread {
        /// The span of the spread token (for use in error messages).
        spread: Token!(...),
        /// The array value that should be spread.
        value: Box<Expr>,
    },
}

/// A pattern in the `somnus` language, which values can be matched against.
#[derive(Facet)]
#[repr(u8)]
pub enum Pat {
    /// A variable declaration, like `a`.
    VariableDefinition(Identifier),
    /// An object, like `{ a, a: b, "a": b, ...c }`.
    Object(PatObject),
    /// A list, like `[a, b, ...c, d]`.
    List(PatList),
    /// A literal, like `"a"`, `0`, `true`, or `None`.
    Literal(Literal),
}

/// An object [`Pat`], like `{ a, a: b, "a": b, ...c }`.
#[derive(Facet)]
pub struct PatObject {
    curly: Token!({}),
    pairs: Vec<PatObjectPair>,
}

#[derive(Facet)]
#[repr(u8)]
pub enum PatObjectPair {
    KeyValue {
        key: Identifier,
        value: Box<Pat>,
    },
    Spread {
        spread: Token!(...),
        value: Box<Pat>,
    },
}

#[derive(Facet)]
pub struct PatList {
    square: Token!([]),
    elements: Vec<PatListElement>,
}

#[derive(Facet)]
#[repr(u8)]
pub enum PatListElement {
    Single(Box<Pat>),
    Spread {
        spread: Token!(...),
        value: Box<Pat>,
    },
}

#[derive(Facet)]
#[repr(u8)]
pub enum Literal {
    None(NoneToken),
    Boolean(BooleanToken),
    Number(NumberToken),
    String(StringToken),
    UUID(UuidToken),
}

pub enum Type {}

#[derive(Facet)]
#[facet(transparent)]
pub struct NoneToken(Span);

#[derive(Facet)]
pub struct BooleanToken {
    value: bool,
    span: Span,
}

#[derive(Facet)]
pub struct NumberToken {
    value: f64,
    span: Span,
}

#[derive(Facet)]
pub struct UuidToken {
    value: uuid::Uuid,
    span: Span,
}

#[derive(Facet)]
pub struct StringToken {
    parts: Vec<StringPart>,
    span: Span,
}

#[derive(Facet)]
#[repr(u8)]
pub enum StringPart {
    String {
        span: Span,
        value: String,
    },
    Expr {
        dollar: Token!($),
        curly: Curly,
        value: Box<Expr>,
    },
}

#[derive(Facet)]
pub struct Identifier {
    value: String,
    span: Span,
}
