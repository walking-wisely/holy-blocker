use super::language::Language;
use super::steps::{common, english};

pub type NormalizationStep = fn(&str) -> String;

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
                common::trim_text,
                common::collapse_whitespace,
                english::lowercase,
            ],
        }
    }

    pub fn ukrainian() -> Self {
        Self {
            language: Language::Ukrainian,
            steps: &[common::trim_text, common::collapse_whitespace],
        }
    }

    pub fn unknown() -> Self {
        Self {
            language: Language::Unknown,
            steps: &[common::trim_text],
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
            Some("Keep\t   Spacing".to_string())
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
            "Keep   Spacing"
        );
    }
}
