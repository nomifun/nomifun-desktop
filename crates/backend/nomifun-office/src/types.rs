use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DocType {
    Word,
    Excel,
    Ppt,
}

impl DocType {
    pub fn event_prefix(&self) -> &'static str {
        match self {
            Self::Word => "word-preview",
            Self::Excel => "excel-preview",
            Self::Ppt => "ppt-preview",
        }
    }

    pub fn proxy_prefix(&self) -> &'static str {
        match self {
            Self::Word | Self::Excel => "office-watch-proxy",
            Self::Ppt => "ppt-proxy",
        }
    }

    pub fn officecli_subcommand(&self) -> &'static str {
        "watch"
    }
}

impl std::fmt::Display for DocType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Word => write!(f, "word"),
            Self::Excel => write!(f, "excel"),
            Self::Ppt => write!(f, "ppt"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OfficecliStatus {
    Starting,
    Installing,
    Ready,
    Error,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doc_type_event_prefix() {
        assert_eq!(DocType::Word.event_prefix(), "word-preview");
        assert_eq!(DocType::Excel.event_prefix(), "excel-preview");
        assert_eq!(DocType::Ppt.event_prefix(), "ppt-preview");
    }

    #[test]
    fn doc_type_proxy_prefix() {
        assert_eq!(DocType::Word.proxy_prefix(), "office-watch-proxy");
        assert_eq!(DocType::Excel.proxy_prefix(), "office-watch-proxy");
        assert_eq!(DocType::Ppt.proxy_prefix(), "ppt-proxy");
    }

    #[test]
    fn doc_type_officecli_subcommand() {
        assert_eq!(DocType::Word.officecli_subcommand(), "watch");
        assert_eq!(DocType::Excel.officecli_subcommand(), "watch");
        assert_eq!(DocType::Ppt.officecli_subcommand(), "watch");
    }

    #[test]
    fn doc_type_display() {
        assert_eq!(DocType::Word.to_string(), "word");
        assert_eq!(DocType::Excel.to_string(), "excel");
        assert_eq!(DocType::Ppt.to_string(), "ppt");
    }

    #[test]
    fn doc_type_serialize() {
        let cases = [
            (DocType::Word, "\"word\""),
            (DocType::Excel, "\"excel\""),
            (DocType::Ppt, "\"ppt\""),
        ];
        for (dt, expected) in cases {
            assert_eq!(serde_json::to_string(&dt).unwrap(), expected);
        }
    }

    #[test]
    fn doc_type_deserialize() {
        let cases = [
            ("\"word\"", DocType::Word),
            ("\"excel\"", DocType::Excel),
            ("\"ppt\"", DocType::Ppt),
        ];
        for (input, expected) in cases {
            let parsed: DocType = serde_json::from_str(input).unwrap();
            assert_eq!(parsed, expected);
        }
    }

    #[test]
    fn doc_type_invalid_deserialize() {
        assert!(serde_json::from_str::<DocType>("\"pdf\"").is_err());
    }

    #[test]
    fn doc_type_eq_and_hash() {
        use std::collections::HashSet;
        let mut set = HashSet::new();
        set.insert(DocType::Word);
        set.insert(DocType::Excel);
        set.insert(DocType::Ppt);
        assert_eq!(set.len(), 3);
        assert!(set.contains(&DocType::Word));
    }

    #[test]
    fn officecli_status_variants() {
        let statuses = [
            OfficecliStatus::Starting,
            OfficecliStatus::Installing,
            OfficecliStatus::Ready,
            OfficecliStatus::Error,
        ];
        assert_eq!(statuses.len(), 4);
        assert_ne!(OfficecliStatus::Starting, OfficecliStatus::Ready);
    }
}
