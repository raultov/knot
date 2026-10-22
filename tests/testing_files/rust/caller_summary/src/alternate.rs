pub fn adjust(sku: &str) -> Vec<u32> {
    compute_value(sku)
}

pub fn preview(sku: &str) -> Vec<u32> {
    compute_value(sku)
}

/// Compute the alternate per-currency adjustment values for a sku.
fn compute_value(sku: &str) -> Vec<u32> {
    vec![sku.len() as u32]
}
