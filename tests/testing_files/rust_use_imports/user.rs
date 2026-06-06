use crate::lib::{BracedTrait};
use crate::lib::AliasedTrait as Renamed;
pub fn touch<T: BracedTrait + Renamed>(t: &T) {}
