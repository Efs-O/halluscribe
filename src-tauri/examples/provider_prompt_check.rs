use app_lib::{
    gemma::{run_inference, InferenceBackend},
    readers::ChatProvider,
    settings::HalluScribeSettings,
};
use std::env;

fn main() {
    let host = env::var("OLLAMA_HOST").unwrap_or_else(|_| "localhost".into());
    let port: u16 = env::var("OLLAMA_PORT")
        .ok()
        .and_then(|value| value.parse().ok())
        .unwrap_or(11434);
    let model = env::var("OLLAMA_MODEL").unwrap_or_else(|_| "gemma4:26b".into());
    let settings = HalluScribeSettings::default();

    let backend = InferenceBackend::Ollama { host, port, model };
    let cases = [
        (
            "Codex",
            ChatProvider::Codex,
            "[User]\nPlease fix the failing auth refresh flow in src/auth.ts. The access token expires early and tests are red.\n\n[Assistant]\nI inspected src/auth.ts and auth.test.ts. The refresh window compared milliseconds against seconds, so refresh triggered too late.\n\n[Assistant]\nI changed src/auth.ts to normalize expiry timestamps to milliseconds, updated auth.test.ts with a regression case for early expiry, and kept the public API unchanged.\n\n[User]\nSummarize what changed and note any remaining risk.",
        ),
        (
            "Claude Chat",
            ChatProvider::ClaudeAI,
            "[User]\nI'm trying to decide whether to leave my current job this summer. I'm torn between financial stability and burnout.\n\n[Assistant]\nWe listed your priorities: reduce stress, keep health insurance, and avoid rushing into a bad role. We discussed staying long enough to rebuild savings and setting a deadline for a decision.\n\n[User]\nI also said I could freelance for a few months if I cut expenses.\n\n[Assistant]\nRight, and we compared three options: stay six more months, quit with a savings runway, or shift to part-time consulting while you search.\n\n[User]\nPlease recap the main tradeoffs and what I still need to decide.",
        ),
    ];

    for (label, provider, transcript) in cases {
        println!("=== {label} / {:?} ===", provider);
        match run_inference(
            &backend,
            settings.ctx_size,
            settings.max_tokens,
            &provider,
            transcript,
        ) {
            Ok(out) => {
                println!("title: {:?}", out.title);
                println!("session_type: {:?}", out.session_type);
                println!("error_tags: {:?}", out.error_tags);
                println!("topic_tags: {:?}", out.topic_tags);
                println!("summary:\n{}\n", out.summary);
            }
            Err(error) => {
                println!("error: {error}\n");
            }
        }
    }
}
