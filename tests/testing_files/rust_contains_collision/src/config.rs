pub struct Config {
    pub name: String,
}

impl Config {
    pub fn load_mcp() -> Self {
        Self {
            name: String::from("mcp"),
        }
    }

    pub fn load_indexer() -> Self {
        Self {
            name: String::from("indexer"),
        }
    }
}
