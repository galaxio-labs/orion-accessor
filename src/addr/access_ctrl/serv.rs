use crate::prelude::*;
use std::{path::PathBuf, rc::Rc};

use orion_conf::YamlIO;

use crate::{
    addr::{
        AddrError, AddrReason, GitRepository, HttpResource,
        access_ctrl::{
            auth::AuthConfig,
            unit::{RedirectResult, Unit},
        },
        proxy::ProxyConfig,
    },
    timeout::TimeoutConfig,
};

use super::{UnitCtrl, rule::Rule};
use serde_derive::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, Getters, PartialEq)]
#[getset(get = "pub")]
pub struct NetAccessCtrl {
    units: Vec<Unit>,
    enable: bool,
}

pub type ServHandle = Rc<NetAccessCtrl>;

impl NetAccessCtrl {
    pub fn new(units: Vec<Unit>, enable: bool) -> Self {
        Self { units, enable }
    }
    pub fn redirect(&self, url: &str) -> RedirectResult {
        let mut path = RedirectResult::Origin(url.to_string());
        for unit in &self.units {
            path = unit.redirect(path.path());
            if path.is_proxy() {
                break;
            }
        }
        path
    }
    pub fn direct_http_addr(&self, origin: HttpResource) -> HttpResource {
        for unit in &self.units {
            if let Some(dirct) = unit.direct_http_addr(&origin) {
                return dirct;
            }
        }
        origin
    }
    pub fn direct_git_addr(&self, origin: GitRepository) -> GitRepository {
        for unit in &self.units {
            if let Some(dirct) = unit.direct_git_addr(&origin) {
                return dirct;
            }
        }
        origin
    }
    pub fn direct_git_ctrl(&self, origin: &GitRepository) -> Option<UnitCtrl> {
        for unit in &self.units {
            if unit.direct_git_addr(origin).is_some() {
                return Some(UnitCtrl::new(
                    unit.auth().clone(),
                    unit.timeout().clone(),
                    unit.proxy().clone(),
                ));
            }
        }
        None
    }
    pub fn proxy_git(&self, origin: &GitRepository) -> Option<ProxyConfig> {
        self.direct_git_ctrl(origin).and_then(|x| x.proxy().clone())
    }
    pub fn proxy_http(&self, origin: &HttpResource) -> Option<ProxyConfig> {
        self.direct_http_ctrl(origin)
            .and_then(|x| x.proxy().clone())
    }

    pub fn timeout_git(&self, origin: &GitRepository) -> Option<TimeoutConfig> {
        self.direct_git_ctrl(origin)
            .and_then(|x| x.timeout().clone())
    }
    pub fn timeout_http(&self, origin: &HttpResource) -> Option<TimeoutConfig> {
        self.direct_http_ctrl(origin)
            .and_then(|x| x.timeout().clone())
    }

    pub fn auth_git(&self, origin: &GitRepository) -> Option<AuthConfig> {
        self.direct_git_ctrl(origin).and_then(|x| x.auth().clone())
    }
    pub fn auth_http(&self, origin: &HttpResource) -> Option<AuthConfig> {
        self.direct_http_ctrl(origin).and_then(|x| x.auth().clone())
    }

    pub fn direct_http_ctrl(&self, origin: &HttpResource) -> Option<UnitCtrl> {
        for unit in &self.units {
            if unit.direct_http_addr(origin).is_some() {
                return Some(UnitCtrl::new(
                    unit.auth().clone(),
                    unit.timeout().clone(),
                    unit.proxy().clone(),
                ));
            }
        }
        None
    }

    pub fn from_rule(rule: Rule, auth: Option<AuthConfig>, proxy: Option<ProxyConfig>) -> Self {
        let unit = Unit::new(vec![rule], auth, proxy);
        Self::new(vec![unit], true)
    }
}
impl TryFrom<&PathBuf> for NetAccessCtrl {
    type Error = AddrError;

    fn try_from(value: &PathBuf) -> Result<Self, Self::Error> {
        NetAccessCtrl::load_yaml(value)
            .map_err(|e| {
                AddrReason::resource_error()
                    .to_err()
                    .with_detail(e.to_string())
            })
            .with_context(value)
    }
}

impl EnvEvalable<NetAccessCtrl> for NetAccessCtrl {
    fn env_eval(self, dict: &EnvDict) -> NetAccessCtrl {
        NetAccessCtrl {
            units: self
                .units
                .into_iter()
                .map(|unit| unit.env_eval(dict))
                .collect(),
            enable: self.enable,
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::addr::proxy::ProxyConfig;

    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_serv_serialization_basic() {
        let serv = NetAccessCtrl::new(vec![], false);
        let serialized = serde_yaml::to_string(&serv).unwrap();
        let deserialized: NetAccessCtrl = serde_yaml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.units().len(), 0);
        assert!(!deserialized.enable());
    }

    #[test]
    fn test_serv_serialization_with_units() {
        let auth = Some(AuthConfig::new("test_user", "test_pass"));
        let rules = vec![
            Rule::new("https://github.com/*", "https://mirror.github.com/"),
            Rule::new("https://gitlab.com/*", "https://mirror.gitlab.com/"),
        ];
        let proxy = ProxyConfig::new("https://proxy.example.com:8080");
        let unit = Unit::new(rules, auth, Some(proxy));
        let serv = NetAccessCtrl::new(vec![unit], true);

        let serialized = serde_json::to_string(&serv).unwrap();
        let deserialized: NetAccessCtrl = serde_json::from_str(&serialized).unwrap();

        assert_eq!(deserialized.units().len(), 1);
        assert!(deserialized.enable());
        assert_eq!(deserialized.units()[0].rules().len(), 2);
        assert!(deserialized.units()[0].auth().is_some());
    }

    #[test]
    fn test_serv_serialization_yaml_format() {
        let yaml_content = r#"
units:
  - rules:
      - pattern: "https://example.com/*"
        target: "https://proxy.com/"
enable: true
"#;

        let deserialized: NetAccessCtrl = serde_yaml::from_str(yaml_content).unwrap();
        assert!(deserialized.enable());
        assert_eq!(deserialized.units().len(), 1);
        assert_eq!(
            deserialized.units()[0].rules()[0].pattern(),
            "https://example.com/*"
        );
    }

    #[test]
    fn test_serv_from_rule_serialization() {
        let rule = Rule::new("https://test.com/*", "https://redirect.com/");
        let auth = Some(AuthConfig::new("admin", "secret"));
        let serv = NetAccessCtrl::from_rule(rule, auth, None);

        let serialized = serde_json::to_string_pretty(&serv).unwrap();
        let deserialized: NetAccessCtrl = serde_json::from_str(&serialized).unwrap();

        assert!(deserialized.enable());
        assert_eq!(deserialized.units().len(), 1);
        assert_eq!(deserialized.units()[0].rules().len(), 1);
        assert!(deserialized.units()[0].auth().is_some());
    }

    #[test]
    fn test_serv_multiple_units_serialization() {
        let unit1 = Unit::new(
            vec![Rule::new("https://api1.com/*", "https://proxy1.com/")],
            Some(AuthConfig::new("user1", "pass1")),
            None,
        );

        let unit2 = Unit::new(
            vec![Rule::new("https://api2.com/*", "https://proxy2.com/")],
            None,
            None,
        );

        let unit3 = Unit::new(
            vec![
                Rule::new("https://api3.com/v1/*", "https://proxy3.com/v1/"),
                Rule::new("https://api3.com/v2/*", "https://proxy3.com/v2/"),
            ],
            Some(AuthConfig::new("user3", "pass3")),
            None,
        );

        let serv = NetAccessCtrl::new(vec![unit1, unit2, unit3], true);

        let serialized = serde_yaml::to_string(&serv).unwrap();
        let deserialized: NetAccessCtrl = serde_yaml::from_str(&serialized).unwrap();

        assert_eq!(deserialized.units().len(), 3);
        assert!(deserialized.enable());
    }

    #[test]
    fn test_serv_complex_yaml_structure() {
        let yaml_content = r#"
units:
  - rules:
      - pattern: "https://github.com/*"
        target: "https://ghproxy.com/"
      - pattern: "https://raw.githubusercontent.com/*"
        target: "https://raw.ghproxy.com/"
    auth:
      username: "proxy_user"
      password: "proxy_pass"
  - rules:
      - pattern: "https://npmjs.com/*"
        target: "https://npmmirror.com/"
      - pattern: "https://registry.npmjs.org/*"
        target: "https://registry.npmmirror.com/"
enable: true
"#;

        let deserialized: NetAccessCtrl = serde_yaml::from_str(yaml_content).unwrap();

        assert_eq!(deserialized.units().len(), 2);
        assert!(deserialized.enable());

        let first_unit = &deserialized.units()[0];
        assert_eq!(first_unit.rules().len(), 2);
        assert_eq!(first_unit.rules()[0].pattern(), "https://github.com/*");
        assert_eq!(first_unit.rules()[0].target(), "https://ghproxy.com/");
        assert!(first_unit.auth().is_some());
        assert_eq!(first_unit.auth().as_ref().unwrap().username(), "proxy_user");

        let second_unit = &deserialized.units()[1];
        assert_eq!(second_unit.rules().len(), 2);
        assert!(second_unit.auth().is_none());
    }

    #[test]
    fn test_serv_empty_yaml() {
        let yaml_content = r#"
units: []
enable: false
"#;

        let deserialized: NetAccessCtrl = serde_yaml::from_str(yaml_content).unwrap();

        assert_eq!(deserialized.units().len(), 0);
        assert!(!deserialized.enable());
    }

    #[test]
    fn test_serv_json_format() {
        let json_content = r#"
{
  "units": [
    {
      "rules": [
        {
          "pattern": "https://test.example.com/*",
          "target": "https://proxy.example.com/"
        }
      ],
      "auth": {
        "username": "testuser",
        "password": "testpass"
      }
    }
  ],
  "enable": true
}
"#;

        let deserialized: NetAccessCtrl = serde_json::from_str(json_content).unwrap();

        assert_eq!(deserialized.units().len(), 1);
        assert!(deserialized.enable());
        assert_eq!(
            deserialized.units()[0].rules()[0].pattern(),
            "https://test.example.com/*"
        );
    }

    #[test]
    fn test_serv_redirect_functionality() {
        let rules = vec![Rule::new("https://github.com/*", "https://mirror.com/")];
        let unit = Unit::new(rules, None, None);
        let serv = NetAccessCtrl::new(vec![unit], true);

        let result = serv.redirect("https://github.com/user/repo");
        match result {
            RedirectResult::Direct(path, _) => {
                assert_eq!(path, "https://mirror.com/user/repo");
            }
            RedirectResult::Origin(_) => panic!("Expected proxy path"),
        }
    }

    #[test]
    fn test_serv_no_redirect_match() {
        let rules = vec![Rule::new("https://github.com/*", "https://mirror.com/")];
        let unit = Unit::new(rules, None, None);
        let serv = NetAccessCtrl::new(vec![unit], true);

        let result = serv.redirect("https://gitlab.com/user/repo");
        match result {
            RedirectResult::Origin(path) => {
                assert_eq!(path, "https://gitlab.com/user/repo");
            }
            RedirectResult::Direct(_, _) => panic!("Expected origin path"),
        }
    }

    #[test]
    fn test_serv_file_roundtrip() {
        let temp_dir = tempdir().unwrap();
        let file_path = temp_dir.path().join("test_serv.yml");

        let rules = vec![Rule::new(
            "https://file-test.com/*",
            "https://file-proxy.com/",
        )];
        let unit = Unit::new(rules, Some(AuthConfig::new("file_user", "file_pass")), None);
        let original_serv = NetAccessCtrl::new(vec![unit], true);

        // 写入文件
        original_serv.save_yaml(&file_path).unwrap();

        // 从文件读取
        let loaded_serv = NetAccessCtrl::try_from(&file_path).unwrap();

        assert_eq!(loaded_serv.units().len(), original_serv.units().len());
        assert_eq!(loaded_serv.enable(), original_serv.enable());
    }

    #[test]
    fn test_redirect_service() {
        let service = NetAccessCtrl::new(
            vec![Unit::new(
                vec![Rule::new(
                    "https://github.com/galaxio-labs/galaxy-flow*",
                    "https://gflow.com",
                )],
                None,
                None,
            )],
            true,
        );
        let result = service.redirect("https://github.com/galaxio-labs/galaxy-flow");
        match result {
            RedirectResult::Direct(path, _) => {
                assert_eq!(path, "https://gflow.com");
            }
            RedirectResult::Origin(_) => panic!("Expected proxy path"),
        }
    }

    #[test]
    fn test_redirect_service_env_eval() {
        let mut env_dict = EnvDict::new();
        env_dict.insert(
            "DOMAIN".to_string(),
            ValueType::String("example.com".to_string()),
        );
        env_dict.insert(
            "TARGET".to_string(),
            ValueType::String("redirect.com".to_string()),
        );
        env_dict.insert(
            "USERNAME".to_string(),
            ValueType::String("test_user".to_string()),
        );

        let service = NetAccessCtrl::new(
            vec![
                Unit::new(
                    vec![Rule::new("https://${DOMAIN}/*", "https://${TARGET}")],
                    None,
                    None,
                ),
                Unit::new(
                    vec![Rule::new(
                        "https://github.com/*",
                        "https://mirror.${DOMAIN}",
                    )],
                    Some(AuthConfig::new("${USERNAME}", "password")),
                    None,
                ),
            ],
            true,
        );

        let evaluated = service.env_eval(&env_dict);

        assert_eq!(evaluated.units().len(), 2);
        assert_eq!(
            evaluated.units()[0].rules()[0].pattern(),
            "https://example.com/*"
        );
        assert_eq!(
            evaluated.units()[0].rules()[0].target(),
            "https://redirect.com"
        );
        assert!(evaluated.units()[0].auth().is_none());

        assert_eq!(
            evaluated.units()[1].rules()[0].pattern(),
            "https://github.com/*"
        );
        assert_eq!(
            evaluated.units()[1].rules()[0].target(),
            "https://mirror.example.com"
        );
        assert!(evaluated.units()[1].auth().is_some());
        assert_eq!(
            evaluated.units()[1].auth().as_ref().unwrap().username(),
            "test_user"
        );
    }

    #[test]
    fn test_redirect_service_env_eval_disabled() {
        let env_dict = EnvDict::new();

        let service = NetAccessCtrl::new(
            vec![Unit::new(
                vec![Rule::new("https://github.com/*", "https://mirror.com")],
                None,
                None,
            )],
            false,
        );

        let evaluated = service.env_eval(&env_dict);

        assert_eq!(evaluated.units().len(), 1);
        assert!(!evaluated.enable());
    }
}
