use std::marker::PhantomData;

pub(crate) type Output<T, E> = Result<T, <<E as Effect>::Fallibility as Fallibility>::Error>;
#[allow(dead_code)]
pub(crate) type CleanOutput<T, E> = <<E as Effect>::Fallibility as Fallibility>::CleanOutput<T>;

#[expect(clippy::unnecessary_wraps)]
pub(crate) const fn pure<T, E: Effect>(value: T) -> Output<T, E> {
    Ok(value)
}

pub(crate) fn extract<T, U, E: Effect>(value: Output<T, E>) -> Result<T, Output<U, E>> {
    match value {
        Ok(val) => Ok(val),
        Err(val) => Err(Err(val)),
    }
}

pub(crate) fn error<T, E: Effect, Err>(error: Err) -> Output<T, E>
where
    Err: Into<<E::Fallibility as Fallibility>::Error>,
{
    Err(error.into())
}

#[allow(dead_code)]
pub(crate) fn clean<T: Send + 'static, E: Effect>(value: Output<T, E>) -> CleanOutput<T, E> {
    E::Fallibility::clean_output(value)
}

// TODO: bring back Kind and Apply, implement Effect for each Kind and its respective trait,
// like EffectImpl<Kind, E>, then (E,) will use EffectImpl, (E, E) will use Apply

pub trait Effect: Send + Sync + 'static {
    type Exclusivity: Exclusivity;
    type Fallibility: Fallibility;
}

pub trait Exclusivity:
    Partial<Use = ExclusivityKind<Self>> + Sized + Send + Sync + 'static
{
    type Level: typenum::Unsigned + typenum::IsGreaterOrEqual<typenum::U0, Output = typenum::B1>;
}

pub trait Fallibility:
    Partial<Use = FallibilityKind<Self>> + Sized + Send + Sync + 'static
{
    type Error: Send + 'static;

    type CleanOutput<T: Send + 'static>: Send + 'static;
    fn clean_output<T: Send + 'static>(value: Result<T, Self::Error>) -> Self::CleanOutput<T>;
}

pub trait Partial: Send + Sync + 'static {
    type Use: Effect;
}

macro_rules! partial {
    ($ty:ty, $kind:ty $(, ($($tt:tt)*))?) => {
        impl $($($tt)*)? Partial for $ty {
            type Use = $kind;
        }
    };
}

pub struct Pure(PhantomData<()>);
pub struct Immutable(PhantomData<()>);
pub struct Mutable(PhantomData<()>);

partial!(Pure, ExclusivityKind<Self>);
partial!(Immutable, ExclusivityKind<Self>);
partial!(Mutable, ExclusivityKind<Self>);

pub struct Infallible(PhantomData<()>);
pub struct Fallible<E>(PhantomData<fn() -> E>);

partial!(Infallible, FallibilityKind<Self>);
partial!(Fallible<E>, FallibilityKind<Self>, (<E: From<!> + Send + 'static>));

impl Exclusivity for Pure {
    type Level = typenum::U0;
}

impl Exclusivity for Immutable {
    type Level = typenum::U1;
}

impl Exclusivity for Mutable {
    type Level = typenum::U2;
}

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

pub struct ExclusivityKind<E: Exclusivity>(PhantomData<E>);
impl<E: Exclusivity> Effect for ExclusivityKind<E> {
    type Exclusivity = E;
    type Fallibility = Infallible;
}

pub struct FallibilityKind<F: Fallibility>(PhantomData<F>);
impl<F: Fallibility> Effect for FallibilityKind<F> {
    type Exclusivity = Pure;
    type Fallibility = F;
}

pub trait ExclusivityContains<E: Exclusivity>:
    Exclusivity<Level: typenum::IsGreaterOrEqual<E::Level, Output = typenum::B1>>
{
}

impl<Outer, Inner> ExclusivityContains<Inner> for Outer
where
    Inner: Exclusivity,
    Outer: Exclusivity,
    Outer::Level: typenum::IsGreaterOrEqual<Inner::Level, Output = typenum::B1>,
{
}

pub trait FallibilityContains<F: Fallibility>: Fallibility<Error: From<F::Error>> {}
impl<Outer, Inner> FallibilityContains<Inner> for Outer
where
    Inner: Fallibility,
    Outer: Fallibility,
    Outer::Error: From<Inner::Error>,
{
}

pub trait Contains<E: Effect>:
    Effect<
        Exclusivity: ExclusivityContains<E::Exclusivity>,
        Fallibility: FallibilityContains<E::Fallibility>,
    >
{
}

impl<Outer, Inner> Contains<Inner> for Outer
where
    Inner: Effect,
    Outer: Effect,
    Outer::Exclusivity: ExclusivityContains<Inner::Exclusivity>,
    Outer::Fallibility: FallibilityContains<Inner::Fallibility>,
{
}

pub trait Apply<Other: Effect>: Effect {
    type And: Effect;
}

impl<A: Effect, O: Effect> Apply<O> for A
where
    A::Exclusivity: ApplyExclusivity<O::Exclusivity>,
    A::Fallibility: ApplyFalliblity<O::Fallibility>,
{
    type And = ApplyImpl<A, O>;
}

pub struct ApplyImpl<A, O>(PhantomData<(A, O)>);
impl<A: Effect, O: Effect> Effect for ApplyImpl<A, O>
where
    A::Exclusivity: ApplyExclusivity<O::Exclusivity>,
    A::Fallibility: ApplyFalliblity<O::Fallibility>,
{
    type Exclusivity = <A::Exclusivity as ApplyExclusivity<O::Exclusivity>>::And;
    type Fallibility = <A::Fallibility as ApplyFalliblity<O::Fallibility>>::And;
}

pub trait ApplyExclusivity<Other: Exclusivity>: Exclusivity {
    type And: Exclusivity;
}

impl<E1: Exclusivity, E2: Exclusivity> ApplyExclusivity<E2> for E1
where
    E1::Level: typenum::Max<
            E2::Level,
            Output: typenum::Unsigned
                        + typenum::IsGreaterOrEqual<typenum::U0, Output = typenum::B1>,
        >,
{
    type And = ApplyExclusivityImpl<E1, E2>;
}

pub struct ApplyExclusivityImpl<E1, E2>(PhantomData<(E1, E2)>);
impl<E1: Exclusivity, E2: Exclusivity> Exclusivity for ApplyExclusivityImpl<E1, E2>
where
    E1::Level: typenum::Max<
            E2::Level,
            Output: typenum::Unsigned
                        + typenum::IsGreaterOrEqual<typenum::U0, Output = typenum::B1>,
        >,
{
    type Level = <E1::Level as typenum::Max<E2::Level>>::Output;
}

impl<E1: Exclusivity, E2: Exclusivity> Partial for ApplyExclusivityImpl<E1, E2>
where
    E1::Level: typenum::Max<
            E2::Level,
            Output: typenum::Unsigned
                        + typenum::IsGreaterOrEqual<typenum::U0, Output = typenum::B1>,
        >,
{
    type Use = ExclusivityKind<Self>;
}

pub trait ApplyFalliblity<Other: Fallibility>: Fallibility {
    type And: Fallibility;
}

impl<F: Fallibility> ApplyFalliblity<F> for Infallible {
    type And = F;
}

impl<F: Fallibility, E: From<!> + Send + 'static> ApplyFalliblity<F> for Fallible<E> {
    type And = Self;
}

impl Effect for () {
    type Exclusivity = Pure;
    type Fallibility = Infallible;
}

impl<E: Partial> Effect for (E,) {
    type Exclusivity = <E::Use as Effect>::Exclusivity;
    type Fallibility = <E::Use as Effect>::Fallibility;
}

impl<E: Partial, F: Partial> Effect for (E, F)
where
    E::Use: Apply<F::Use>,
{
    type Exclusivity = <<E::Use as Apply<F::Use>>::And as Effect>::Exclusivity;
    type Fallibility = <<E::Use as Apply<F::Use>>::And as Effect>::Fallibility;
}
