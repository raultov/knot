//! Pipeline module.
//!
//! Each sub-module implements one stage of the indexing pipeline:
//!
//! | Stage | Module     | Description                                        |
//! |-------|------------|----------------------------------------------------|
//! | 1     | `input`    | Discover `.java` / `.ts` / `.tsx` / `.cts` source files |
//! | 2     | `parse`    | Extract entities from ASTs via Tree-sitter + Rayon |
//! | 3     | `prepare`  | Assign UUIDs and build embedding text              |
//! | 4     | `embed`    | Generate vectors with fastembed                    |
//! | 5     | `ingest`   | Dual-write to Qdrant and Neo4j                     |

pub mod embed;
pub mod ingest;
pub mod input;
pub mod parse;
pub mod prepare;
pub mod state;
