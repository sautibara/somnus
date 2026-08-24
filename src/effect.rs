use std::marker::PhantomData;

pub trait Effect: sealed::Sealed + Send + Sync + 'static {
    type Exclusivity: Exclusivity;
    type Fallibility: Fallibility;
}

pub trait Exclusivity: sealed::Sealed + Send + Sync + 'static {}

pub struct Pure;
pub struct Immutable;
pub struct Mutable;

pub trait Fallibility: sealed::Sealed + Send + Sync + 'static {
    type MapOutput<Output: Send + 'static>: Send + 'static;

    #[doc(hidden)]
    fn pure<T: Send + 'static>(value: T) -> Self::MapOutput<T>;
    #[doc(hidden)]
    fn extract<T: Send + 'static, O: Send + 'static>(
        output: Self::MapOutput<T>,
    ) -> Result<T, Self::MapOutput<O>>;
}

pub struct Infallible;
pub struct Fallible<Err: Send + 'static>(PhantomData<fn() -> Err>);

pub trait Contains<E: Effect>:
    Effect<
        Exclusivity: ExclusivityContains<E::Exclusivity>,
        Fallibility: FallibilityContains<E::Fallibility>,
    >
{
}

pub trait ExclusivityContains<E: Exclusivity>: Exclusivity {}
pub trait FallibilityContains<E: Fallibility>: Fallibility {
    #[doc(hidden)]
    fn map_output<T: Send + 'static>(input: E::MapOutput<T>) -> Self::MapOutput<T>;
}

pub type DefaultExclusivity = Pure;
pub type DefaultFallibility = Infallible;

impl Exclusivity for Pure {}
impl Exclusivity for Immutable {}
impl Exclusivity for Mutable {}

impl Fallibility for Infallible {
    type MapOutput<Output: Send + 'static> = Output;

    fn pure<T: Send + 'static>(value: T) -> Self::MapOutput<T> {
        value
    }

    fn extract<T: Send + 'static, O: Send + 'static>(
        output: Self::MapOutput<T>,
    ) -> Result<T, Self::MapOutput<O>> {
        Ok(output)
    }
}

impl<Err: Send + 'static> Fallibility for Fallible<Err> {
    type MapOutput<Output: Send + 'static> = Result<Output, Err>;

    fn pure<T: Send + 'static>(value: T) -> Self::MapOutput<T> {
        Ok(value)
    }

    fn extract<T: Send + 'static, O: Send + 'static>(
        output: Self::MapOutput<T>,
    ) -> Result<T, Self::MapOutput<O>> {
        output.map_err(|err| Err(err))
    }
}

mod sealed {
    pub trait Sealed {}
    impl Sealed for super::Pure {}
    impl Sealed for super::Immutable {}
    impl Sealed for super::Mutable {}

    impl Sealed for super::Infallible {}
    impl<Err: Send + 'static> Sealed for super::Fallible<Err> {}

    impl<E: super::Exclusivity, F: super::Fallibility> Sealed for (E, F) {}
    impl Sealed for () {}
}

impl Effect for Pure {
    type Exclusivity = Self;
    type Fallibility = DefaultFallibility;
}

impl Effect for Immutable {
    type Exclusivity = Self;
    type Fallibility = DefaultFallibility;
}

impl Effect for Mutable {
    type Exclusivity = Self;
    type Fallibility = DefaultFallibility;
}

impl Effect for Infallible {
    type Exclusivity = DefaultExclusivity;
    type Fallibility = Self;
}

impl<Err: Send + 'static> Effect for Fallible<Err> {
    type Exclusivity = DefaultExclusivity;
    type Fallibility = Self;
}

impl Effect for () {
    type Exclusivity = DefaultExclusivity;
    type Fallibility = DefaultFallibility;
}

impl<E: Exclusivity, F: Fallibility> Effect for (E, F) {
    type Exclusivity = E;
    type Fallibility = F;
}

impl ExclusivityContains<Self> for Pure {}

impl ExclusivityContains<Pure> for Immutable {}
impl ExclusivityContains<Self> for Immutable {}

impl ExclusivityContains<Pure> for Mutable {}
impl ExclusivityContains<Immutable> for Mutable {}
impl ExclusivityContains<Self> for Mutable {}

impl FallibilityContains<Self> for Infallible {
    fn map_output<T: Send + 'static>(input: Self::MapOutput<T>) -> Self::MapOutput<T> {
        input
    }
}

impl<Err: Send + 'static> FallibilityContains<Infallible> for Fallible<Err> {
    fn map_output<T: Send + 'static>(
        input: <Infallible as Fallibility>::MapOutput<T>,
    ) -> Self::MapOutput<T> {
        Self::pure(input)
    }
}

impl<E1, E2> FallibilityContains<Fallible<E1>> for Fallible<E2>
where
    E1: Send + 'static,
    E2: From<E1> + Send + 'static,
{
    fn map_output<T: Send + 'static>(
        input: <Fallible<E1> as Fallibility>::MapOutput<T>,
    ) -> Self::MapOutput<T> {
        input.map_err(Into::into)
    }
}

impl<E1, E2> Contains<E2> for E1
where
    E1: Effect,
    E2: Effect,
    E1::Exclusivity: ExclusivityContains<E2::Exclusivity>,
    E1::Fallibility: FallibilityContains<E2::Fallibility>,
{
}
