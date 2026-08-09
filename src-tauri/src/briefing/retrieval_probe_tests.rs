// HalluScribe - manual live probe: can the briefing assistant actually retrieve
// a comparison board that IS in the archive? Replays the 2026-08-04 failure.
//
// v2 (2026-08-04). The v1 probe reported 0/4 and was used to conclude that no
// prompt change could work. That conclusion was unsafe, because v1 had two
// measurement bugs, both now fixed:
//   1. `reached_target` was "the id appeared anywhere in a tool output", which a
//      search-result LIST satisfies trivially — a run could score a hit without
//      the model ever opening the session. It now requires read_session /
//      read_raw_session to have been called WITH that id.
//   2. Every question ran in a fresh conversation, so the multi-turn shape of
//      the real bug (turn 2 finds the session, turn 3 forgets it) was never
//      exercised. Q2 is now a genuine two-turn conversation.
//
// Ground truth, verified against the live archive on 2026-08-04:
//   search_raw_transcripts("beats me")        -> HIT, session 3d13a20e, line 546
//   search_raw_transcripts("where gemma wins") -> 0 hits (paraphrase, not verbatim)
//   search_sessions("beats me")                -> 0 of 1726 (summariser dropped it)
// So the data is reachable and the TOOLS work. What failed is the model's choice
// of queries. That is what this probe measures.
//
//   HALLUSCRIBE_PROBE_ARCHIVE="C:\Users\<you>\.halluscribe" \
//     cargo test --release retrieval_prompt_probe -- --ignored --nocapture
//
// Reads llama-server bin/model/gpu/ctx from that archive's settings.json.
// Uses its own port so it cannot collide with a running HalluScribe.

use super::chat::ChatRuntimeOptions;
use super::llamacpp;
use super::tools::{self, ChatScope, ToolCallResult};
use crate::chat_prompt::{build_chat_system_prompt, ChatPromptContext, SearchModePrompt};
use crate::profile::ProfileScope;
use crate::settings;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

/// Port deliberately away from the configured one (8091/8092) so this probe
/// never fights a running app for a port or a GPU slot.
const PROBE_PORT: u16 = 8099;

/// Max tool round-trips per user turn before we call the attempt a wash.
const MAX_TURNS: usize = 14;

/// The session that actually holds the board, and verbatim fragments from it.
const TARGET_SESSION: &str = "3d13a20e-e043-4946-b015-2396b073a37e";
const BOARD_FRAGMENTS: [&str; 2] = ["beats me", "who wins"];

/// v1's proposed strategy: escalate to raw search and retry with synonyms.
/// Kept as a control. Its weak point is the synonym-guessing rule — no rule
/// makes a model guess "who wins" from "where gemma wins".
const STRATEGY_SYNONYM: &str = "\n\n\
Raw transcript strategy:\n\
- The distilled summaries are lossy. A term missing from `search_sessions` results is NOT evidence the user never said it. Escalate to `search_raw_transcripts`, which searches the verbatim preserved transcripts.\n\
- `search_raw_transcripts` matches a LITERAL case-insensitive substring. Never pass the user's whole sentence. Pass the shortest distinctive fragment that would plausibly appear verbatim.\n\
- If a literal query returns nothing, retry with alternate terminology before reporting absence. Try at least two alternate phrasings.\n\
- Only state that something does not exist after you have escalated to raw search and tried alternate wordings.";

/// The strategy this probe exists to test. Its core claim is that guessing
/// substrings is the wrong game: identify the SESSION first (cheap, tolerant,
/// keyword-based), then OPEN it and read. A paraphrase like "where gemma wins"
/// will never match a table heading of "| task | who wins |", but the session
/// holding that table is trivially findable by its topic words — and once you
/// are inside the session, no guessing is required at all.
const STRATEGY_READ_FIRST: &str = "\n\n\
Archive retrieval strategy:\n\
- The distilled summaries are LOSSY. A term missing from `search_sessions` results is NOT evidence the user never said it. Tables, verdicts and comparison boards are frequently dropped from the summary while surviving in the preserved raw transcript.\n\
- Locate the SESSION before you chase the WORDING. `search_sessions` is keyword-based and tolerant: search its topic (\"VLM prompt pipeline\", \"gemma benchmark\") rather than the exact phrase the user used.\n\
- Once a session is plausibly the right one, OPEN IT with `read_raw_session` and read. Do not keep issuing searches. `read_raw_session` is paged by byte offset: when `truncated` is true, call again with `offset` set to `next_offset` until you have found the part you need, or have paged to the end.\n\
- This matters because `search_raw_transcripts` matches a LITERAL case-insensitive substring, with no tokenization and no AND/OR. The user's paraphrase almost never matches the transcript's wording: a user asking \"where gemma wins\" is looking for a table whose heading reads \"who wins\". Guessing that substring is not a strategy. Reading the session is.\n\
- Use `search_raw_transcripts` only for strings you have a specific reason to believe appear verbatim — an exact error message, a filename, a command, or a phrase the user themselves quoted.\n\
- A session you already identified earlier in THIS conversation stays identified. If a later question is about that session, go straight back to `read_raw_session` on its id. A hit in some other session tells you nothing about the one you already found.\n\
- Only state that something is absent after you have opened the relevant session and read it. When you do, say which session you read and which queries you tried, and note that sessions without a preserved raw cannot be raw-searched at all.";

struct Question {
    label: &'static str,
    /// Consecutive user turns in ONE conversation. The live bug was multi-turn:
    /// an earlier turn identified the session and a later turn forgot it, so a
    /// single-turn probe cannot reproduce it.
    turns: &'static [&'static str],
}

const QUESTIONS: [Question; 2] = [
    Question {
        label: "Q1 vocabulary-mismatch",
        turns: &[
            "in the raw transcripts there is a discussion where claude admits that gemma4 12b \
             is better at image recognition - its from august 2026 - can you find it?",
        ],
    },
    Question {
        label: "Q2 board-in-known-session",
        turns: &[
            "what do you know about the local VLM prompt pipeline validation session \
             from august 2026?",
            "in that same session there is a board that says where gemma wins and where not - \
             find it and quote it to me",
        ],
    },
];

#[derive(Default)]
struct Attempt {
    tool_calls: Vec<String>,
    /// The id merely appeared in some tool output (e.g. a search result list).
    /// This is what v1 wrongly scored as success.
    seen_in_output: bool,
    /// The model actually OPENED the target session. This is the real metric.
    opened_target: bool,
    /// The final answer quotes board content the summary does not contain.
    quoted_board: bool,
    answer: String,
}

#[test]
#[ignore = "manual live probe; set HALLUSCRIBE_PROBE_ARCHIVE"]
fn retrieval_prompt_probe() {
    let Ok(dir) = std::env::var("HALLUSCRIBE_PROBE_ARCHIVE") else {
        eprintln!("HALLUSCRIBE_PROBE_ARCHIVE not set; skipping");
        return;
    };
    let archive_dir = PathBuf::from(dir);
    let cfg = settings::load_settings(&archive_dir);
    let bin = PathBuf::from(&cfg.llama_server_bin);
    let model = PathBuf::from(&cfg.gemma_model_path);
    assert!(bin.exists(), "llama_server_bin missing: {}", bin.display());
    assert!(
        model.exists(),
        "gemma_model_path missing: {}",
        model.display()
    );
    assert!(
        cfg.ctx_size > 0 && cfg.max_tokens > 0,
        "ctx_size/max_tokens are 0 in settings.json — generation cannot run"
    );

    let base_prompt = build_chat_system_prompt(&ChatPromptContext {
        web_search_available: false,
        has_images: false,
        search_mode: SearchModePrompt::Archive,
        scope_size: None,
        profile: None,
        profile_scope: ProfileScope::Work,
    });
    let synonym_prompt = format!("{base_prompt}{STRATEGY_SYNONYM}");
    let read_first_prompt = format!("{base_prompt}{STRATEGY_READ_FIRST}");

    println!("\n=== HalluScribe - retrieval prompt probe (v2) ===");
    println!("archive : {}", archive_dir.display());
    println!("model   : {}", model.display());
    println!("target  : {TARGET_SESSION}");
    println!("metric  : opened_target = read_session/read_raw_session CALLED WITH that id\n");

    let variants = [
        ("CURRENT", &base_prompt),
        ("SYNONYM", &synonym_prompt),
        ("READ_FIRST", &read_first_prompt),
    ];
    let mut summary: Vec<(String, String, Attempt)> = Vec::new();

    for (variant, prompt) in variants {
        for question in &QUESTIONS {
            println!("--- {variant} / {} ---", question.label);
            let attempt = run_attempt(&archive_dir, &cfg, &bin, &model, prompt, question.turns);
            println!("  tools    : {}", attempt.tool_calls.join(" -> "));
            println!(
                "  seen={} opened={} quoted={}",
                attempt.seen_in_output, attempt.opened_target, attempt.quoted_board
            );
            println!("  answer   : {}\n", first_chars(&attempt.answer, 400));
            summary.push((variant.to_string(), question.label.to_string(), attempt));
        }
    }

    println!("\n=== verdict ===");
    println!(
        "{:<11} {:<26} {:>6} {:>7} {:>7}  tool path",
        "variant", "question", "seen", "opened", "quoted"
    );
    for (variant, label, attempt) in &summary {
        println!(
            "{:<11} {:<26} {:>6} {:>7} {:>7}  {}",
            variant,
            label,
            attempt.seen_in_output,
            attempt.opened_target,
            attempt.quoted_board,
            attempt.tool_calls.join(" -> ")
        );
    }
    for (variant, _) in variants {
        let runs: Vec<&Attempt> = summary
            .iter()
            .filter(|(v, _, _)| v == variant)
            .map(|(_, _, a)| a)
            .collect();
        println!(
            "{variant:<11} opened {}/{}  quoted {}/{}",
            runs.iter().filter(|a| a.opened_target).count(),
            runs.len(),
            runs.iter().filter(|a| a.quoted_board).count(),
            runs.len()
        );
    }
    super::server::kill_server();
    println!("\n(probe server unloaded)");
}

/// One conversation, possibly several user turns, against one system prompt.
/// Mirrors `briefing::chat::run_chat_turn`'s loop shape without its
/// AppHandle/event plumbing.
fn run_attempt(
    archive_dir: &Path,
    cfg: &settings::HalluScribeSettings,
    bin: &Path,
    model: &Path,
    system_prompt: &str,
    user_turns: &[&str],
) -> Attempt {
    let runtime = ChatRuntimeOptions {
        chat_scope: ChatScope::ArchiveWide,
        web_search_enabled: false,
        ollama_api_key: None,
        tavily_api_key: None,
        reasoning_enabled: false,
    };
    let tools = tools::chat_tools(&runtime);
    let mut messages = vec![json!({"role": "system", "content": system_prompt})];
    let cancel = AtomicBool::new(false);
    let mut attempt = Attempt::default();

    for (index, user_turn) in user_turns.iter().enumerate() {
        if index > 0 {
            attempt.tool_calls.push("|| next user turn ||".to_string());
        }
        messages.push(json!({"role": "user", "content": user_turn}));
        attempt.answer = drive_turn(
            archive_dir,
            &runtime,
            cfg,
            bin,
            model,
            &tools,
            &mut messages,
            &cancel,
            &mut attempt.tool_calls,
            &mut attempt.seen_in_output,
            &mut attempt.opened_target,
        );
        messages.push(json!({"role": "assistant", "content": attempt.answer}));
    }
    // Only the FINAL answer is scored for board content: that is the answer the
    // user would actually read.
    let lowered = attempt.answer.to_lowercase();
    attempt.quoted_board = BOARD_FRAGMENTS.iter().any(|f| lowered.contains(f));
    attempt
}

/// Run the tool loop for a single user turn; returns the assistant's text.
#[allow(clippy::too_many_arguments)]
fn drive_turn(
    archive_dir: &Path,
    runtime: &ChatRuntimeOptions,
    cfg: &settings::HalluScribeSettings,
    bin: &Path,
    model: &Path,
    tools: &[Value],
    messages: &mut Vec<Value>,
    cancel: &AtomicBool,
    tool_calls: &mut Vec<String>,
    seen_in_output: &mut bool,
    opened_target: &mut bool,
) -> String {
    for _ in 0..MAX_TURNS {
        let result = llamacpp::stream(
            bin,
            model,
            PROBE_PORT,
            &cfg.gpu_config(),
            cfg.ctx_size,
            cfg.max_tokens,
            false,
            messages,
            tools,
            |_token, _done| {},
            cancel,
        );
        match result {
            Ok(ToolCallResult::ToolCall { id, name, args, .. }) => {
                tool_calls.push(describe_call(&name, &args));
                // The real metric: the model OPENED the target session, rather
                // than merely receiving its id inside a list of search results.
                if matches!(name.as_str(), "read_session" | "read_raw_session")
                    && args["session_id"].as_str() == Some(TARGET_SESSION)
                {
                    *opened_target = true;
                }
                let output = tools::execute_tool(archive_dir, runtime, &name, &args);
                if output.contains(TARGET_SESSION) {
                    *seen_in_output = true;
                }
                messages.push(json!({
                    "role": "assistant",
                    "tool_calls": [{
                        "id": id,
                        "type": "function",
                        "function": { "name": name, "arguments": args.to_string() }
                    }]
                }));
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": id,
                    "content": output,
                }));
            }
            Ok(ToolCallResult::Text { text, .. }) => return text,
            Err(error) => return format!("[stream error: {error}]"),
        }
    }
    format!("[no text answer within {MAX_TURNS} tool round-trips]")
}

/// Tool name plus the argument that actually determines whether the call can
/// succeed — the query string — so the printed path shows the model's search
/// wording, not just which tool it reached for.
fn describe_call(name: &str, args: &Value) -> String {
    let detail = args
        .get("query")
        .or_else(|| args.get("session_id"))
        .and_then(Value::as_str)
        .map(|value| format!("({})", first_chars(value, 60)))
        .unwrap_or_default();
    format!("{name}{detail}")
}

fn first_chars(text: &str, max: usize) -> String {
    let cleaned = text.replace('\n', " ");
    if cleaned.chars().count() <= max {
        return cleaned;
    }
    cleaned.chars().take(max).collect::<String>() + "…"
}
