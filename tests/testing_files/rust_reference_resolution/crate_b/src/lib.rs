use crate_a::config::Config as CrateAConfig;

pub struct Config {
    pub local_field: String,
}

impl Config {
    pub fn local_new() -> Self {
        Self {
            local_field: String::from("local"),
        }
    }
}

pub fn use_crate_a_config() -> CrateAConfig {
    CrateAConfig::load().unwrap()
}
