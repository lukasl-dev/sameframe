use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(version, about = "A cosy, private watch-together server")]
pub(crate) struct Cli {
    /// Allow visitors without a site access token (room permissions still apply)
    #[arg(long, conflicts_with = "access_token_file")]
    pub public: bool,

    /// Persistent site token file, generated if missing [default: ./access-token]
    #[arg(long, value_name = "PATH")]
    pub access_token_file: Option<PathBuf>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn private_by_default_with_an_optional_token_path() {
        let defaults = Cli::try_parse_from(["sameframe"]).unwrap();
        assert!(!defaults.public);
        assert!(defaults.access_token_file.is_none());

        let custom = Cli::try_parse_from([
            "sameframe",
            "--access-token-file",
            "/var/lib/sameframe/access-token",
        ])
        .unwrap();
        assert_eq!(
            custom.access_token_file.unwrap(),
            PathBuf::from("/var/lib/sameframe/access-token")
        );
    }

    #[test]
    fn public_mode_is_explicit_and_rejects_conflicting_or_unknown_options() {
        assert!(
            Cli::try_parse_from(["sameframe", "--public"])
                .unwrap()
                .public
        );
        assert!(
            Cli::try_parse_from(["sameframe", "--public", "--access-token-file", "key"]).is_err()
        );
        assert!(Cli::try_parse_from(["sameframe", "--access-token-file"]).is_err());
        assert!(Cli::try_parse_from(["sameframe", "--publc"]).is_err());
    }
}
