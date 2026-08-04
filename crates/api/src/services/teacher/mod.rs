//! Teacher-model configuration for distillation.
//!
//! The teacher is the external LLM that writes a distilled model's training
//! examples. Everything that touches teacher config goes through this module:
//! policy classification (`policy`), and validation / key encryption /
//! provenance shaping (`config`). No other code reads or writes the
//! `teacher` blocks in request DTOs, `datasets.config`, or
//! `training_jobs.teacher_config`.

pub mod config;
pub mod policy;
