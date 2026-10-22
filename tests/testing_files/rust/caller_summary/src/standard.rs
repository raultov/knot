pub fn adjust(sku: &str) -> u32 {
    compute_value(sku)
}

/// Compute the standard price adjustment value for a sku.
fn compute_value(sku: &str) -> u32 {
    sku.len() as u32
}
