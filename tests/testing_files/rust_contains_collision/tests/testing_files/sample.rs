pub struct Config {
    pub test_value: i32,
}

impl Config {
    pub fn new() -> Self {
        Self { test_value: 0 }
    }
}
