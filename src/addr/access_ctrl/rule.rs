use crate::prelude::*;
use serde_derive::{Deserialize, Serialize};
use wildmatch::WildMatch;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Getters)]
#[getset(get = "pub")]
pub struct Rule {
    #[serde(skip)]
    matchs: WildMatch,
    pattern: String,
    target: String,
}

impl Rule {
    pub fn new<S: AsRef<str>, S2: Into<String>>(matchs: S, target: S2) -> Self {
        let pattern = matchs.as_ref().to_string();
        Self {
            matchs: WildMatch::new(&pattern),
            pattern,
            target: target.into(),
        }
    }
    pub fn replace(&self, input: &str) -> Option<String> {
        if self.matchs.matches(input) {
            // 找到模式中的通配符位置
            if let Some(star_idx) = self.pattern.find('*') {
                let prefix = &self.pattern[..star_idx];
                if let Some(suffix) = input.strip_prefix(prefix) {
                    return Some(format!("{}{suffix}", self.target));
                }
            }
            // 如果没有通配符或者精确匹配，直接替换整个字符串
            None
        } else {
            None
        }
    }
}
impl EnvEvalable<Rule> for Rule {
    fn env_eval(self, dict: &EnvDict) -> Rule {
        Rule::new(self.pattern.env_eval(dict), self.target.env_eval(dict))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_rule2() {
        let rule = Rule::new(
            "https://github.com/galaxio-labs/galaxy-flow*",
            "https://gflow.com",
        );
        let url = rule.replace("https://github.com/galaxio-labs/galaxy-flow/releases/download/v0.8.5/galaxy-flow-v0.8.5-aarch64-apple-darwin.tar.gz");
        assert_eq!(url, Some("https://gflow.com/releases/download/v0.8.5/galaxy-flow-v0.8.5-aarch64-apple-darwin.tar.gz".to_string()));
    }

    #[test]
    fn test_rule_env_eval() {
        let mut env_dict = EnvDict::new();
        env_dict.insert(
            "DOMAIN".to_string(),
            ValueType::String("example.com".to_string()),
        );
        env_dict.insert(
            "TARGET".to_string(),
            ValueType::String("redirect.com".to_string()),
        );

        let rule = Rule::new("https://${DOMAIN}/*", "https://${TARGET}");
        let evaluated = rule.env_eval(&env_dict);

        assert_eq!(evaluated.pattern(), "https://example.com/*");
        assert_eq!(evaluated.target(), "https://redirect.com");
    }

    #[test]
    fn test_rule_env_eval_with_defaults() {
        let env_dict = EnvDict::new();

        let rule = Rule::new(
            "https://${MISSING_DOMAIN:default.com}/*",
            "https://${MISSING_TARGET:target.com}",
        );
        let evaluated = rule.env_eval(&env_dict);

        assert_eq!(evaluated.pattern(), "https://default.com/*");
        assert_eq!(evaluated.target(), "https://target.com");
    }
}
