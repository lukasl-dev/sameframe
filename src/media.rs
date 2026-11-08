use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Deserialize, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub(crate) enum MediaSource {
    Youtube { id: String },
    Direct { url: String },
}

impl MediaSource {
    pub(crate) fn is_valid(&self) -> bool {
        match self {
            Self::Youtube { id } => {
                id.len() == 11
                    && id
                        .bytes()
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
            }
            Self::Direct { url } => {
                if url.len() > 2048
                    || !url.bytes().all(|byte| {
                        byte.is_ascii_graphic()
                            && !matches!(byte, b'"' | b'\\' | b'<' | b'>' | b'#')
                    })
                {
                    return false;
                }
                let Ok(parsed) = url::Url::parse(url) else {
                    return false;
                };
                let path = parsed.path().to_ascii_lowercase();
                parsed.as_str() == url
                    && matches!(parsed.scheme(), "http" | "https")
                    && parsed.has_host()
                    && parsed.username().is_empty()
                    && parsed.password().is_none()
                    && [".mp4", ".webm", ".ogv", ".ogg", ".m4v", ".mov"]
                        .iter()
                        .any(|extension| path.ends_with(extension))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn media_sources_share_the_browser_validation_corpus() {
        let cases: Vec<serde_json::Value> =
            serde_json::from_str(include_str!("../tests/media-sources.json")).unwrap();
        for case in cases {
            let valid = serde_json::from_value::<MediaSource>(case["source"].clone())
                .is_ok_and(|source| source.is_valid());
            assert_eq!(valid, case["valid"].as_bool().unwrap(), "{case}");
        }
        assert!(
            !MediaSource::Direct {
                url: format!("https://example.org/a.mp4?{}", "a".repeat(2048))
            }
            .is_valid()
        );
    }
}
