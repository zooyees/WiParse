//! Provider-reported usage only. Missing usage is unknown, never zero-cost inference.
use super::*;
#[derive(Default, Clone, Serialize)]
pub struct Usage {
    pub responses: u64,
    pub reported_responses: u64,
    pub failed_attempts: u64,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub cached_input_tokens: Option<u64>,
    pub total_tokens: Option<u64>,
}
fn add(sum: &mut Option<u64>, n: Option<u64>) {
    if let Some(n) = n {
        *sum = Some(sum.unwrap_or(0).saturating_add(n));
    }
}
impl Usage {
    pub fn record(&mut self, u: &Value) {
        self.responses = self.responses.saturating_add(1);
        let cached = u["prompt_cache_hit_tokens"]
            .as_u64()
            .or_else(|| u["prompt_tokens_details"]["cached_tokens"].as_u64())
            .or_else(|| u["input_tokens_details"]["cached_tokens"].as_u64());
        let input = u["prompt_tokens"]
            .as_u64()
            .or_else(|| u["input_tokens"].as_u64())
            .or_else(|| {
                cached
                    .zip(u["prompt_cache_miss_tokens"].as_u64())
                    .map(|(a, b)| a.saturating_add(b))
            });
        let output = u["completion_tokens"]
            .as_u64()
            .or_else(|| u["output_tokens"].as_u64());
        let total = u["total_tokens"]
            .as_u64()
            .or_else(|| input.zip(output).map(|(a, b)| a.saturating_add(b)));
        if input.is_some() || output.is_some() || total.is_some() {
            self.reported_responses = self.reported_responses.saturating_add(1);
        }
        add(&mut self.input_tokens, input);
        add(&mut self.output_tokens, output);
        add(&mut self.cached_input_tokens, cached);
        add(&mut self.total_tokens, total);
    }
    pub fn failure(&mut self) {
        self.failed_attempts = self.failed_attempts.saturating_add(1);
    }
    pub fn summary(&self) -> String {
        let n = |v: Option<u64>| v.map_or_else(|| "?".into(), |n| n.to_string());
        format!(
            "tokens · in {} · out {} · cache {} · usage {}/{} · unknown failures {}",
            n(self.input_tokens),
            n(self.output_tokens),
            n(self.cached_input_tokens),
            self.reported_responses,
            self.responses,
            self.failed_attempts
        )
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn deepseek_qwen_and_responses_usage_are_normalized_without_double_counting_cache() {
        let mut u = Usage::default();
        u.record(
            &json!({"prompt_tokens":1000,"completion_tokens":100,"prompt_cache_hit_tokens":800}),
        );
        u.record(&json!({"prompt_tokens":2000,"completion_tokens":200,"prompt_tokens_details":{"cached_tokens":1000}}));
        u.record(&json!({"input_tokens":300,"output_tokens":30,"input_tokens_details":{"cached_tokens":0}}));
        assert_eq!(u.input_tokens, Some(3300));
        assert_eq!(u.output_tokens, Some(330));
        assert_eq!(u.cached_input_tokens, Some(1800));
        assert_eq!(u.total_tokens, Some(3630));
        assert_eq!(u.reported_responses, 3);
    }
    #[test]
    fn absent_or_failed_responses_never_claim_zero_cost() {
        let mut u = Usage::default();
        u.record(&Value::Null);
        u.failure();
        assert_eq!(u.input_tokens, None);
        assert_eq!(u.total_tokens, None);
        assert_eq!(u.reported_responses, 0);
        assert_eq!(u.responses, 1);
        assert!(u.summary().contains('?'));
    }
}
