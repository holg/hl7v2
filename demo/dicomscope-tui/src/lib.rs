#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(clippy::unwrap_used, clippy::expect_used, missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod app;
pub mod cli;
pub mod commands;
pub mod gpu;
pub mod term;
pub mod ui;
pub mod window;

#[cfg(test)]
mod tests;
