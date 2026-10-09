use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(version, about = "A cosy, private watch-together server")]
pub(crate) struct Cli {
    #[arg(
        long,
        conflicts_with = "access_token_file",
        help = "Allow visitors without a site access token (room permissions still apply)"
    )]
    pub public: bool,

    #[arg(
        long,
        value_name = "PATH",
        help = "Persistent site token file, generated if missing [default: ./access-token]"
    )]
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
    fn help_describes_public_mode_and_token_storage() {
        use clap::CommandFactory;

        let mut command = Cli::command();
        for help in [command.render_help(), command.render_long_help()] {
            let help = help.to_string();
            assert!(help.contains(
                "Allow visitors without a site access token (room permissions still apply)"
            ));
            assert!(help.contains(
                "Persistent site token file, generated if missing [default: ./access-token]"
            ));
        }
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
