// HalluScribe — pure logic: curve data, context window lookup.
// curves.json is embedded at compile time so this module has no runtime I/O.

use serde::Deserialize;

// --- Curve data structures --------------------------------------------------

#[derive(Deserialize, Debug)]
pub struct CurvePoint {
    pub fill_pct: f64,
    pub risk_score: f64,
}

#[derive(Deserialize, Debug)]
pub struct ModelCurve {
    pub id: String,
    pub context_window: u64,
    pub degradation_curve: Vec<CurvePoint>,
}

#[derive(Deserialize, Debug)]
pub struct CurvesConfig {
    pub models: Vec<ModelCurve>,
}

// Embedded at compile time — single source of truth.
static CURVES_JSON: &str = include_str!("../assets/curves.json");

pub fn load_curves() -> CurvesConfig {
    serde_json::from_str(CURVES_JSON).expect("curves.json is malformed")
}

// --- Helpers ----------------------------------------------------------------

/// Context window in tokens for `model`.
/// Returns 200 000 if the model is not in curves.json.
pub fn context_window_for_model(model: &str) -> u64 {
    load_curves()
        .models
        .iter()
        .find(|m| m.id == model)
        .map(|m| m.context_window)
        .unwrap_or(200_000)
}

// --- Tests ------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn curves_json_loads() {
        let cfg = load_curves();
        assert!(
            !cfg.models.is_empty(),
            "curves.json must contain at least one model"
        );
    }

    #[test]
    fn unknown_model_falls_back_to_200k() {
        assert_eq!(context_window_for_model("not-a-real-model-xyz"), 200_000);
    }

    #[test]
    fn known_model_returns_nonzero_window() {
        // Any known model must have a positive context window.
        let cfg = load_curves();
        let first = &cfg.models[0];
        assert!(context_window_for_model(&first.id) > 0);
    }
}
