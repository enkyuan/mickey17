//! Declaration-to-class translation: per-element Tailwind utilities (with responsive and state
//! variants) and shared recipes, with a cascade-based equality proof. See `docs/TOKENS.md`.
pub mod classes;
pub mod emit;
pub mod layers;
pub mod run;
pub mod tailwind;
pub mod verify;
