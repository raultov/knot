//! Pipeline module.
//!
//! Each sub-module implements one stage of the indexing pipeline:
//!
//! | Stage | Module       | Description                                        |
//! |-------|----------|----------------------------------------------------|
//! | 0     | `runner`     | Orchestrates all pipeline stages                   |
//! | 1     | `input`    | Discover `.java` / `.ts` / `.tsx` / `.cts` source files |
//! | 2     | `parser`   | Extract entities from ASTs via Tree-sitter + Rayon |
//! | 3     | `prepare`  | Assign UUIDs and build embedding text              |
//! | 4     | `embed`    | Generate vectors with fastembed                    |
//! | 5     | `ingest`   | Dual-write to Qdrant and Neo4j                     |

pub mod embed;
pub mod ingest;
pub mod input;
pub mod parser;
pub mod prepare;
pub mod runner;
pub mod state;

#[cfg(test)]
mod tests {
    #[test]
    fn test_pipeline_module_structure() {
        // Simple test to ensure all modules are accessible
    }
}
