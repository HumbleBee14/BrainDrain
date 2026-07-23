//! Ollama Modelfile generation for exported GGUF models.
//!
//! Turns a completed GGUF export into a copy-paste "run locally with Ollama"
//! recipe: a Modelfile plus the `ollama` commands to build and run it. Pure —
//! no I/O — so it is fully unit tested; the service layer resolves the export,
//! the base model, and the download URL.
//!
//! The Modelfile deliberately does NOT emit a chat `TEMPLATE`. llama.cpp's
//! GGUF conversion embeds the source model's `tokenizer.chat_template` into the
//! GGUF metadata, and Ollama applies it automatically — so the served prompt
//! format matches what the model was trained under (train/serve consistency).
//! Emitting a hand-written TEMPLATE here would risk overriding that with a
//! wrong one. We only add the model-family stop tokens (which Ollama does not
//! always infer) and the trained system prompt.

/// Stop tokens for a base-model family. Ollama does not always infer these from
/// GGUF metadata, and without them generation can run past the turn boundary.
fn stop_tokens(base_model: &str) -> &'static [&'static str] {
    let b = base_model.to_lowercase();
    if b.contains("llama-3") || b.contains("llama3") {
        &["<|eot_id|>", "<|end_of_text|>"]
    } else if b.contains("qwen") {
        &["<|im_end|>"]
    } else if b.contains("mistral") || b.contains("mixtral") {
        &["</s>"]
    } else if b.contains("phi") {
        &["<|end|>"]
    } else if b.contains("gemma") {
        &["<end_of_turn>"]
    } else {
        // ChatML fallback — matches the platform's fallback chat template.
        &["<|im_end|>"]
    }
}

/// A suggested Ollama model name for an export, e.g. `braindrain-1a2b3c4d-q5-k-m`.
/// Ollama names allow `[a-z0-9._-]`; anything else is folded to `-`.
pub fn model_name(model_id: &str, quant_type: &str) -> String {
    let short = model_id.chars().take(8).collect::<String>();
    let raw = format!("braindrain-{short}-{quant_type}");
    raw.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-') {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// Build the Modelfile text for a GGUF export.
///
/// `gguf_filename` is the name the user will save the downloaded GGUF as, next
/// to the Modelfile (`FROM ./<file>`). A non-empty `system_prompt` is embedded
/// so the local model defaults to the same instruction the model was trained
/// under.
pub fn build_modelfile(base_model: &str, gguf_filename: &str, system_prompt: &str) -> String {
    let mut lines = vec![
        format!("# Generated for a model fine-tuned from {base_model}."),
        "# The chat template is read from the GGUF metadata by Ollama.".to_string(),
        format!("FROM ./{gguf_filename}"),
    ];

    let system_prompt = system_prompt.trim();
    if !system_prompt.is_empty() {
        // Ollama SYSTEM takes a triple-quoted block for multi-line content.
        lines.push(format!("SYSTEM \"\"\"{system_prompt}\"\"\""));
    }

    for stop in stop_tokens(base_model) {
        lines.push(format!("PARAMETER stop \"{stop}\""));
    }

    let mut content = lines.join("\n");
    content.push('\n');
    content
}

/// Build the copy-paste command steps for running the export in Ollama.
pub fn build_instructions(model_name: &str, gguf_filename: &str) -> Vec<String> {
    vec![
        format!("Download the GGUF and save it as '{gguf_filename}'."),
        "Save the Modelfile above next to it (same folder).".to_string(),
        format!("Build the model:  ollama create {model_name} -f Modelfile"),
        format!("Run it:  ollama run {model_name}"),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn model_name_is_sanitized_and_lowercased() {
        let name = model_name("1A2B3C4D-e5f6-7890-abcd-ef0123456789", "Q5_K_M");
        assert_eq!(name, "braindrain-1a2b3c4d-q5_k_m");
        // No uppercase, no illegal chars.
        assert!(
            name.chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        );
    }

    #[test]
    fn modelfile_has_from_line_with_local_path() {
        let mf = build_modelfile("Qwen/Qwen2.5-7B", "model-abc-Q5_K_M.gguf", "");
        assert!(mf.contains("FROM ./model-abc-Q5_K_M.gguf"));
        assert!(mf.ends_with('\n'));
    }

    #[test]
    fn modelfile_omits_system_when_empty() {
        let mf = build_modelfile("Qwen/Qwen2.5-7B", "m.gguf", "   ");
        assert!(!mf.contains("SYSTEM"));
    }

    #[test]
    fn modelfile_includes_system_when_present() {
        let mf = build_modelfile("Qwen/Qwen2.5-7B", "m.gguf", "You are a support agent.");
        assert!(mf.contains("SYSTEM \"\"\"You are a support agent.\"\"\""));
    }

    #[test]
    fn does_not_emit_a_template_line() {
        // The chat template must come from GGUF metadata, never a hand-written
        // TEMPLATE that could override it with the wrong format.
        let mf = build_modelfile("meta-llama/Llama-3.1-8B-Instruct", "m.gguf", "hi");
        assert!(!mf.contains("TEMPLATE"));
    }

    #[test]
    fn stop_tokens_are_family_specific() {
        let llama = build_modelfile("meta-llama/Llama-3.1-8B", "m.gguf", "");
        assert!(llama.contains("PARAMETER stop \"<|eot_id|>\""));

        let qwen = build_modelfile("Qwen/Qwen2.5-7B-Instruct", "m.gguf", "");
        assert!(qwen.contains("PARAMETER stop \"<|im_end|>\""));

        let mistral = build_modelfile("mistralai/Mistral-7B-v0.3", "m.gguf", "");
        assert!(mistral.contains("PARAMETER stop \"</s>\""));

        let gemma = build_modelfile("google/gemma-2-9b", "m.gguf", "");
        assert!(gemma.contains("PARAMETER stop \"<end_of_turn>\""));

        let phi = build_modelfile("microsoft/Phi-3-mini", "m.gguf", "");
        assert!(phi.contains("PARAMETER stop \"<|end|>\""));
    }

    #[test]
    fn unknown_family_falls_back_to_chatml_stop() {
        let mf = build_modelfile("some/unknown-model", "m.gguf", "");
        assert!(mf.contains("PARAMETER stop \"<|im_end|>\""));
    }

    #[test]
    fn instructions_reference_the_name_and_file() {
        let steps = build_instructions("braindrain-abc-q5-k-m", "model-abc-Q5_K_M.gguf");
        assert!(
            steps
                .iter()
                .any(|s| s.contains("ollama create braindrain-abc-q5-k-m -f Modelfile"))
        );
        assert!(
            steps
                .iter()
                .any(|s| s.contains("ollama run braindrain-abc-q5-k-m"))
        );
        assert!(steps.iter().any(|s| s.contains("model-abc-Q5_K_M.gguf")));
    }
}
