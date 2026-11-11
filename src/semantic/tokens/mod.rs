//! Design-token pipeline: CSS AST -> cascade -> typed token model -> one central styles file.
//!
//! Entry points: [`pipeline::run`] (capture directory in, files and report out), [`cli`]
//! (`mochi tokens`), [`plan::TokenSet::token_for`] (property/value lookup for emitters).
//! Documented in `docs/tokens.md`.
pub mod bridge;
pub mod capture;
pub mod cascade;
pub mod cli;
pub mod css;
pub mod emit;
pub mod eval;
pub mod inventory;
pub mod media;
pub mod pipeline;
pub mod plan;
pub mod recognize;
pub mod rewrite;
pub mod selector;
pub mod styles;
pub mod value;
pub mod verify;
