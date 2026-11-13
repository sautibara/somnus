use facet::Facet;
use thiserror::Error;

// NOTE: I'm hoping that [`Data`] won't have to be added as a component.
// Even the different [`Data`] implementations for the standard library are already a lot.
// This will probably be fine as long as we always store undo/redo actions as Facet values.

pub trait Data: Facet<'static> + Send + Sync + 'static {}
impl<D: Facet<'static> + Send + Sync + 'static> Data for D {}

#[derive(Facet)]
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
