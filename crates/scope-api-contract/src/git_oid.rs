use serde::{Deserialize, Deserializer, Serialize, de};
use std::{fmt, ops::Deref};

#[derive(Clone, Debug, Eq, Hash, PartialEq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[cfg_attr(feature = "ts", ts(type = "string"))]
pub struct GitOid(String);

#[cfg(feature = "ts")]
impl schemars::JsonSchema for GitOid {
    fn schema_name() -> std::borrow::Cow<'static, str> {
        "GitOid".into()
    }

    fn json_schema(_generator: &mut schemars::SchemaGenerator) -> schemars::Schema {
        schemars::json_schema!({
            "type": "string",
            "pattern": "^[0-9a-fA-F]{40}$"
        })
    }
}

impl GitOid {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GitOidParseError;

impl fmt::Display for GitOidParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("Git OID must be exactly 40 hexadecimal characters")
    }
}

impl std::error::Error for GitOidParseError {}

impl TryFrom<&str> for GitOid {
    type Error = GitOidParseError;

    fn try_from(value: &str) -> Result<Self, Self::Error> {
        if value.len() != 40 || !value.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(GitOidParseError);
        }
        Ok(Self(value.to_ascii_lowercase()))
    }
}

impl TryFrom<String> for GitOid {
    type Error = GitOidParseError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::try_from(value.as_str())
    }
}

impl<'de> Deserialize<'de> for GitOid {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value).map_err(de::Error::custom)
    }
}

impl From<GitOid> for String {
    fn from(value: GitOid) -> Self {
        value.0
    }
}

impl Deref for GitOid {
    type Target = str;

    fn deref(&self) -> &Self::Target {
        self.as_str()
    }
}

impl fmt::Display for GitOid {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn git_oid_accepts_and_normalizes_canonical_sha1() {
        let oid = GitOid::try_from("ABCDEF0123456789ABCDEF0123456789ABCDEF01").unwrap();
        assert_eq!(oid.as_str(), "abcdef0123456789abcdef0123456789abcdef01");
        assert_eq!(
            serde_json::to_string(&oid).unwrap(),
            "\"abcdef0123456789abcdef0123456789abcdef01\""
        );
    }

    #[test]
    fn git_oid_rejects_non_sha1_values_at_construction_and_deserialization() {
        assert!(GitOid::try_from("head-1").is_err());
        assert!(GitOid::try_from(" aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_err());
        assert!(GitOid::try_from("abcdef0123456789abcdef0123456789abcdef0g").is_err());
        assert!(serde_json::from_str::<GitOid>("\"head-1\"").is_err());
    }
}
