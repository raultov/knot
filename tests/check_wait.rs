use qdrant_client::qdrant::UpsertPointsBuilder;
fn main() {
    let _ = UpsertPointsBuilder::new("col", vec![]).wait(true);
}
