pub struct Config {
    pub name: String,
    pub value: i32,
}

impl Config {
    pub fn load() -> Result<Self, String> {
        Ok(Self {
            name: String::from("default"),
            value: 42,
        })
    }

    pub fn process(cfg: &Config) -> i32 {
        cfg.value
    }
}
