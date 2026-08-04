//! Teacher-model configuration for distillation.
//!
//! The teacher is the external LLM that writes a distilled model's training
//! examples. Everything that touches teacher config goes through this module:
//! policy classification (`policy`), and validation / key encryption /
//! provenance shaping (`config`). No other code reads or writes the
//! `teacher` blocks in request DTOs, `datasets.config`, or
//! `training_jobs.teacher_config`.
//!
//! A teacher that the platform runs on its own GPUs — the only kind whose
//! token-level distributions can be read — lives in `hosted`, with `fidelity`
//! deciding when such a teacher is available for a given dataset and student,
//! `cost` pricing the GPU time that costs, and `billing` capping it.

pub mod billing;
pub mod config;
pub mod cost;
pub mod extraction;
pub mod fidelity;
pub mod hosted;
pub mod policy;
