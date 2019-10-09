use crate::ecs::prelude::*;
use smallvec::SmallVec;
#[derive(Debug, Clone)]
pub struct Children(pub(crate) SmallVec<[Entity; 8]>);

impl Children {
    pub fn with(entity: &[Entity]) -> Self {
        Self(SmallVec::from_slice(entity))
    }
}