//! Command-line options.
//!
//! Every option is optional and every default reproduces the previously
//! hardcoded behaviour, so `holy-blocker-macd`'s `ProxySupervisor` — which
//! spawns this binary with no arguments — keeps working untouched.
//!
//! Hand-rolled rather than pulling in a parser: three flags do not justify a
//! dependency in a binary that sits in the path of all browser traffic.

use std::net::SocketAddr;
use std::path::PathBuf;

pub const USAGE: &str = "\
usage: mitm-proxy [options]

  --listen <addr>        address to bind (default 127.0.0.1:8080)
  --ca-dir <path>        certificate authority directory (default data/ca)
  --image-model <path>   ONNX image classifier; images are not scanned without it
  --image-threshold <f>  block at or above this explicit score; required with
                          --image-model, no built-in default
  --image-sexy-threshold <f>
                          warn at or above this sexy score; required with
                          --image-model, no built-in default
  -h, --help             print this message";

#[derive(Debug, Clone, PartialEq)]
pub struct Options {
    pub listen: SocketAddr,
    pub ca_dir: PathBuf,
    /// `None` means images pass through unscanned — the pre-Phase-4 behaviour.
    pub image_model: Option<PathBuf>,
    /// Explicit score at or above which an image is blocked. Has no built-in
    /// default — a threshold belongs to a model **and** a geometry, and there
    /// is no value this crate can supply that is correct for every deployed
    /// checkpoint. Required whenever `image_model` is set; meaningless (and
    /// unread) otherwise.
    pub image_threshold: Option<f32>,
    /// Sexy score at or above which an image warns. Same no-built-in-default
    /// rule as `image_threshold`, and required alongside it whenever
    /// `image_model` is set.
    pub image_sexy_threshold: Option<f32>,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            listen: "127.0.0.1:8080".parse().expect("literal address is valid"),
            ca_dir: PathBuf::from("data/ca"),
            image_model: None,
            image_threshold: None,
            image_sexy_threshold: None,
        }
    }
}

impl Options {
    pub fn from_args<I: IntoIterator<Item = String>>(args: I) -> Result<Self, String> {
        let mut options = Self::default();
        let mut args = args.into_iter();

        while let Some(flag) = args.next() {
            let mut value = || {
                args.next().ok_or_else(|| format!("{flag} requires a value"))
            };
            match flag.as_str() {
                "--listen" => {
                    let raw = value()?;
                    options.listen =
                        raw.parse().map_err(|e| format!("invalid --listen {raw:?}: {e}"))?;
                }
                "--ca-dir" => options.ca_dir = PathBuf::from(value()?),
                "--image-model" => options.image_model = Some(PathBuf::from(value()?)),
                "--image-threshold" => {
                    let raw = value()?;
                    let parsed: f32 = raw
                        .parse()
                        .map_err(|e| format!("invalid --image-threshold {raw:?}: {e}"))?;
                    if !(0.0..=1.0).contains(&parsed) {
                        return Err(format!(
                            "--image-threshold must be in [0, 1], got {parsed}"
                        ));
                    }
                    options.image_threshold = Some(parsed);
                }
                "--image-sexy-threshold" => {
                    let raw = value()?;
                    let parsed: f32 = raw
                        .parse()
                        .map_err(|e| format!("invalid --image-sexy-threshold {raw:?}: {e}"))?;
                    if !(0.0..=1.0).contains(&parsed) {
                        return Err(format!(
                            "--image-sexy-threshold must be in [0, 1], got {parsed}"
                        ));
                    }
                    options.image_sexy_threshold = Some(parsed);
                }
                "-h" | "--help" => {
                    println!("{USAGE}");
                    std::process::exit(0);
                }
                other => return Err(format!("unknown option {other:?}")),
            }
        }
        if options.image_model.is_some() && options.image_threshold.is_none() {
            return Err(
                "--image-threshold is required when --image-model is set; there is no built-in \
                 default"
                    .to_string(),
            );
        }
        if options.image_model.is_some() && options.image_sexy_threshold.is_none() {
            return Err(
                "--image-sexy-threshold is required when --image-model is set; there is no \
                 built-in default"
                    .to_string(),
            );
        }
        Ok(options)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Options, String> {
        Options::from_args(args.iter().map(|s| s.to_string()))
    }

    #[test]
    fn no_arguments_reproduces_the_previously_hardcoded_configuration() {
        // ProxySupervisor spawns this binary bare. If these defaults drift, the
        // macOS daemon points the system proxy at a port nothing is listening
        // on, which black-holes all traffic.
        let options = parse(&[]).unwrap();

        assert_eq!(options.listen.to_string(), "127.0.0.1:8080");
        assert_eq!(options.ca_dir, PathBuf::from("data/ca"));
        assert_eq!(options.image_model, None);
        assert_eq!(options.image_threshold, None);
        assert_eq!(options.image_sexy_threshold, None);
    }

    #[test]
    fn a_threshold_outside_the_unit_interval_is_rejected() {
        // A score is a probability. Accepting 20 (meaning "20%") would silently
        // disable blocking, since no score can ever reach it.
        assert!(parse(&["--image-threshold", "20"]).is_err());
        assert!(parse(&["--image-threshold", "-0.1"]).is_err());
    }

    #[test]
    fn a_sexy_threshold_outside_the_unit_interval_is_rejected() {
        assert!(parse(&["--image-sexy-threshold", "20"]).is_err());
        assert!(parse(&["--image-sexy-threshold", "-0.1"]).is_err());
    }

    #[test]
    fn a_threshold_is_parsed_when_valid() {
        // Paired with --image-model: a bare --image-threshold with no model is
        // accepted by the parser (and simply unread), which this covers by
        // itself rather than through the full validation path.
        assert_eq!(
            parse(&["--image-threshold", "0.44"]).unwrap().image_threshold,
            Some(0.44)
        );
    }

    #[test]
    fn a_sexy_threshold_is_parsed_when_valid() {
        assert_eq!(
            parse(&["--image-sexy-threshold", "0.3"]).unwrap().image_sexy_threshold,
            Some(0.3)
        );
    }

    #[test]
    fn a_model_without_a_threshold_is_rejected() {
        // There is no built-in default to fall back to, and silently picking
        // one would bake a specific model's calibration into every deployment.
        assert!(parse(&["--image-model", "/tmp/model.onnx"]).is_err());
    }

    #[test]
    fn a_model_with_an_explicit_threshold_but_no_sexy_threshold_is_rejected() {
        assert!(
            parse(&["--image-model", "/tmp/model.onnx", "--image-threshold", "0.5"]).is_err()
        );
    }

    #[test]
    fn images_are_unscanned_unless_a_model_is_named() {
        assert_eq!(parse(&[]).unwrap().image_model, None);
    }

    #[test]
    fn it_parses_each_option() {
        let options = parse(&[
            "--listen",
            "127.0.0.1:9999",
            "--ca-dir",
            "/tmp/ca",
            "--image-model",
            "/tmp/model.onnx",
            "--image-threshold",
            "0.5",
            "--image-sexy-threshold",
            "0.3",
        ])
        .unwrap();

        assert_eq!(options.listen.to_string(), "127.0.0.1:9999");
        assert_eq!(options.ca_dir, PathBuf::from("/tmp/ca"));
        assert_eq!(options.image_model, Some(PathBuf::from("/tmp/model.onnx")));
        assert_eq!(options.image_threshold, Some(0.5));
        assert_eq!(options.image_sexy_threshold, Some(0.3));
    }

    #[test]
    fn a_malformed_listen_address_is_rejected_rather_than_silently_defaulted() {
        // Falling back to the default here would bind a port the caller did not
        // ask for and report success.
        assert!(parse(&["--listen", "not-an-address"]).is_err());
    }

    #[test]
    fn a_flag_without_its_value_is_rejected() {
        assert!(parse(&["--ca-dir"]).is_err());
    }

    #[test]
    fn an_unknown_flag_is_rejected() {
        assert!(parse(&["--wat"]).is_err());
    }
}
