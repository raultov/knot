//! Cross-module homonym trap: a second `PriceAdjuster` with the same private
//! helper. Each `self.compute_value()` must bind to its own module's type.

pub struct PriceAdjuster;

impl PriceAdjuster {
    pub fn recompute(&self, sku: &str) -> u32 {
        self.compute_value(sku)
    }

    fn compute_value(&self, sku: &str) -> u32 {
        sku.len() as u32 * 2
    }
}
