use super::language::Language;
use super::steps::{common, english};

pub type NormalizationStep = fn(&str) -> String;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NormalizedText {
    pub raw: String,
    pub normalized: String,
    pub separator_tokens: Vec<String>,
    pub compact: String,
    pub leet_compact: String,
}

#[derive(Clone, Copy, Debug)]
pub struct NormalizationPipeline {
    language: Language,
    steps: &'static [NormalizationStep],
}

impl NormalizationPipeline {
    pub fn for_language(language: Language) -> Self {
        match language {
            Language::English => Self::english(),
            Language::Ukrainian => Self::ukrainian(),
            Language::Unknown => Self::unknown(),
        }
    }

    pub fn english() -> Self {
        Self {
            language: Language::English,
            steps: &[
                common::normalize_nfkc,
                common::trim_text,
                common::collapse_whitespace,
                english::lowercase,
            ],
        }
    }

    pub fn ukrainian() -> Self {
        Self {
            language: Language::Ukrainian,
            steps: &[
                common::normalize_nfkc,
                common::trim_text,
                common::collapse_whitespace,
            ],
        }
    }

    pub fn unknown() -> Self {
        Self {
            language: Language::Unknown,
            steps: &[
                common::normalize_nfkc,
                common::trim_text,
                common::collapse_whitespace,
            ],
        }
    }

    pub fn from_steps(language: Language, steps: &'static [NormalizationStep]) -> Self {
        Self { language, steps }
    }

    pub fn language(&self) -> Language {
        self.language
    }

    pub fn steps(&self) -> &[NormalizationStep] {
        self.steps
    }

    pub fn normalize(&self, text: &str) -> String {
        self.steps
            .iter()
            .fold(text.to_string(), |current, step| step(&current))
    }

    pub fn normalize_views(&self, text: &str) -> NormalizedText {
        let normalized = self.normalize(text);
        let compact = common::compact(&normalized);
        let leet_compact = match self.language {
            Language::English => english::leet_compact(&normalized),
            Language::Ukrainian | Language::Unknown => compact.clone(),
        };

        NormalizedText {
            raw: text.to_string(),
            separator_tokens: common::separator_tokens(&normalized),
            normalized,
            compact,
            leet_compact,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct NormalizationPipelines {
    pipelines: [NormalizationPipeline; Language::ALL.len()],
}

impl NormalizationPipelines {
    pub fn new() -> Self {
        Self {
            pipelines: [
                NormalizationPipeline::english(),
                NormalizationPipeline::ukrainian(),
                NormalizationPipeline::unknown(),
            ],
        }
    }

    pub fn pipeline_for(&self, language: Language) -> Option<&NormalizationPipeline> {
        self.pipelines
            .iter()
            .find(|pipeline| pipeline.language() == language)
    }

    pub fn normalize(&self, language: Language, text: &str) -> Option<String> {
        self.pipeline_for(language)
            .map(|pipeline| pipeline.normalize(text))
    }

    pub fn normalize_views(&self, language: Language, text: &str) -> Option<NormalizedText> {
        self.pipeline_for(language)
            .map(|pipeline| pipeline.normalize_views(text))
    }
}

impl Default for NormalizationPipelines {
    fn default() -> Self {
        Self::new()
    }
}

pub fn normalize_text(text: &str) -> String {
    common::trim_text(text)
}

pub fn normalize_text_for_language(text: &str, language: Language) -> String {
    NormalizationPipeline::for_language(language).normalize(text)
}

pub fn normalize_text_views_for_language(text: &str, language: Language) -> NormalizedText {
    NormalizationPipeline::for_language(language).normalize_views(text)
}

pub fn normalization_pipeline_for(language: Language) -> NormalizationPipeline {
    NormalizationPipeline::for_language(language)
}

pub fn default_normalization_pipelines() -> NormalizationPipelines {
    NormalizationPipelines::default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn surround_with_brackets(text: &str) -> String {
        format!("[{text}]")
    }

    fn replace_spaces(text: &str) -> String {
        text.replace(' ', "_")
    }

    #[test]
    fn default_pipelines_normalize_each_language() {
        let pipelines = NormalizationPipelines::default();

        assert_eq!(
            pipelines.normalize(Language::English, "  Hello\t   WORLD  "),
            Some("hello world".to_string())
        );
        assert_eq!(
            pipelines.normalize(Language::Ukrainian, "  Привіт\t   світе  "),
            Some("Привіт світе".to_string())
        );
        assert_eq!(
            pipelines.normalize(Language::Unknown, "  Keep\t   Spacing  "),
            Some("Keep Spacing".to_string())
        );
    }

    #[test]
    fn named_pipeline_runs_steps_in_order_with_previous_step_output_only() {
        static STEPS: &[NormalizationStep] =
            &[common::trim_text, surround_with_brackets, replace_spaces];
        let pipeline = NormalizationPipeline::from_steps(Language::Unknown, STEPS);

        assert_eq!(pipeline.normalize("  first second  "), "[first_second]");
    }

    #[test]
    fn pipeline_for_language_returns_the_language_specific_pipeline() {
        assert_eq!(
            NormalizationPipeline::for_language(Language::English).normalize("  Hello WORLD  "),
            "hello world"
        );
        assert_eq!(
            NormalizationPipeline::for_language(Language::Ukrainian)
                .normalize("  Привіт   світе  "),
            "Привіт світе"
        );
        assert_eq!(
            NormalizationPipeline::for_language(Language::Unknown).normalize("  Keep   Spacing  "),
            "Keep Spacing"
        );
    }

    #[test]
    fn english_pipeline_builds_all_text_views_from_normalized_text() {
        assert_eq!(
            NormalizationPipeline::english().normalize_views("  P-0.R_N  H U B  "),
            NormalizedText {
                raw: "  P-0.R_N  H U B  ".to_string(),
                normalized: "p-0.r_n h u b".to_string(),
                separator_tokens: vec![
                    "p".to_string(),
                    "0".to_string(),
                    "r".to_string(),
                    "n".to_string(),
                    "h".to_string(),
                    "u".to_string(),
                    "b".to_string(),
                ],
                compact: "p0rnhub".to_string(),
                leet_compact: "pornhub".to_string(),
            }
        );
    }

    #[test]
    fn non_english_pipeline_uses_compact_as_leet_compact() {
        assert_eq!(
            NormalizationPipeline::ukrainian().normalize_views("  Привіт-світе 123  "),
            NormalizedText {
                raw: "  Привіт-світе 123  ".to_string(),
                normalized: "Привіт-світе 123".to_string(),
                separator_tokens: vec![
                    "Привіт".to_string(),
                    "світе".to_string(),
                    "123".to_string(),
                ],
                compact: "Привітсвіте123".to_string(),
                leet_compact: "Привітсвіте123".to_string(),
            }
        );
    }
}
