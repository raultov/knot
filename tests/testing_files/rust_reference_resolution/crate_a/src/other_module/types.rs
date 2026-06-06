pub struct Config {
    pub other_field: String,
}

impl Config {
    pub fn new() -> Self {
        Self {
            other_field: String::from("other"),
        }
    }
}
