#![doc = include_str!("../README.md")]
#![forbid(unsafe_code)]
#![warn(clippy::unwrap_used, clippy::expect_used, missing_docs)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod app;
pub mod cli;
pub mod commands;
pub mod gpu;
pub mod ui;

#[cfg(test)]
mod tests;
