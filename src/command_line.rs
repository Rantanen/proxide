use clap::{value_parser, Arg, ArgAction, ArgGroup, Command};

pub fn setup_app(version: &str) -> Command
{
    Command::new("Proxide - HTTP2 debugging proxy")
        .next_display_order(None)
        .version(version.to_string())
        .author("Mikko Rantanen <rantanen@jubjubnest.net>")
        // clap 4 doesn't show the author by default; keep the clap 3 layout.
        // `next_display_order(None)` keeps options sorted alphabetically, as in clap 3.
        .help_template(
            "{name} {version}\n{author}\n\n{usage-heading} {usage}\n\n{all-args}{after-help}",
        )
        .subcommand_required(true)
        .arg_required_else_help(true)
        .arg(
            Arg::new("log")
                .long("log")
                .value_name("file")
                .global(true)
                .help("Write a diagnostic log into the specified file")
                .long_help(
                    "\
Write a diagnostic log into the specified file. The log contains details that are not shown in the
UI, such as the reasons why client or server connections failed. Useful for troubleshooting TLS
interception issues.",
                ),
        )
        .arg(
            Arg::new("log-level")
                .long("log-level")
                .value_name("level")
                .global(true)
                .requires("log")
                .value_parser(["error", "warn", "info", "debug", "trace"])
                .default_value("debug")
                .help("Specify the log level used with --log"),
        )
        .subcommand(
            Command::new("view")
                .next_display_order(None)
                .about("View traffic from a session or capture file")
                .long_about(
                    "\
View a previously saved session or captured traffic in the Proxide UI.",
                )
                .json_options()
                .decoder_options()
                .arg(
                    Arg::new("file")
                        .index(1)
                        .value_name("file")
                        .required(true)
                        .help("Specify the file to load"),
                ),
        )
        // Monitor subcommand.
        .subcommand(
            Command::new("monitor")
                .next_display_order(None)
                .about("Monitor network traffic using the Proxide UI")
                .long_about(
                    "\
Monitors network traffic using the Proxide UI. The UI displays all decoded traffic in real time.

The monitoring session can also be exported into a file for later analysis.",
                )
                .connection_options()
                .json_options()
                .decoder_options(),
        )
        // Capture subcommand.
        .subcommand(
            Command::new("capture")
                .next_display_order(None)
                .about("Capture network traffic into a file for later analysis")
                .long_about(
                    "\
Capture network traffic directly into a file. The file can be later analyzed with the 'view'
command. The traffic is written to the output file in real time. This allows capturing traffic over
long periods with the only limit being the disk usage.",
                )
                .connection_options()
                .json_options()
                .arg(
                    Arg::new("file")
                        .short('f')
                        .value_name("file")
                        .help("Specify the output file. Defaults to 'capture-<timestamp>.bin'."),
                ),
        )
        // The config subcommands.
        .subcommand(
            Command::new("config")
                .next_display_order(None)
                .about("Manage Proxide configuration")
                .subcommand_required(true)
                .arg_required_else_help(true)
                // The "config ca" subcommand.
                .subcommand(
                    Command::new("ca")
                        .next_display_order(None)
                        .about("Manage CA certificates required for debugging TLS traffic")
                        .cert_options(false)
                        .arg(
                            Arg::new("create")
                                .action(ArgAction::SetTrue)
                                .long("create")
                                .help("Create a new CA certificate.")
                                .long_help(
                                    "\
Creates a new CA certificate. Proxide will require a CA certificate for intercepting TLS traffic.
The CA certificate is used to sign certificates generated on the fly and must be trusted by the
clients for them to accept these certificates.",
                                ),
                        )
                        .arg(
                            Arg::new("force")
                                .action(ArgAction::SetTrue)
                                .short('f')
                                .long("force")
                                .requires("create")
                                .help("Overwrite existing files.")
                                .long_help(
                                    "\
Allows --create to overwrite existing files.",
                                ),
                        )
                        .arg(
                            Arg::new("revoke")
                                .long("revoke")
                                .num_args(0..=1)
                                .value_name("store")
                                .help(
                                    "\
Revokes existing Proxide CA certificates from platform store",
                                )
                                .long_help(
                                    "\
Revokes all existing Proxide CA certificates from the platform truted CA certificate store.

An optional value can be used to specify whether to revoke the certificates from the user-level or
system-level store a user-level or system-level store. Changes to the system-level store require
administrative privileges.",
                                ),
                        )
                        .arg(
                            Arg::new("trust")
                                .long("trust")
                                .num_args(0..=1)
                                .value_name("store")
                                .conflicts_with("revoke")
                                .help(
                                    "\
Imports the current Proxide CA certificate to the platform store",
                                )
                                .long_help(
                                    "\
Imports the current Proxide CA certificate to the platform certificate store.

Trusting a new certificate will automatically remove the previous certificates from the platform
certificate store as having multiple certificates in the store may cause issues in using any of
them. Use --revoke To remove the current certificate from the platform certificate store without
trusting a new one.

An optional value can be used to specify whether to import the certificate to a user-level or
system-level store. Changes to the system-level store require administrative privileges.

WARNING: If the Proxide CA certificate is imported to the platform certificate store, it can be
used to generate trusted certificates for ANY application that uses the platform certificate store
to validate certificates. In this case the private key should be kept safe to avoid compromising
the system security. It is recommende to revoke the Proxide CA certificate with the --revoke
command when it is not needed anymore.",
                                ),
                        )
                        .arg(
                            Arg::new("duration")
                                .long("duration")
                                .default_value_if("create", "true", Some("7"))
                                .requires("create")
                                .value_parser(value_parser!(u32))
                                .help(
                                    "\
The number of days the new CA certificate is valid. Defaults to 7 days.",
                                )
                                .long_help(
                                    "\
Specifies the number of days the new CA certificate is valid. Defaults to 7 days.

The ASN.1 format does not support arbitrary large dates so the user-specified value is
automatically capped to 2000 years.",
                                ),
                        )
                        .group(
                            ArgGroup::new("action")
                                .args(["create", "revoke", "trust"])
                                .multiple(true)
                                .required(true),
                        ),
                ),
        )
}

trait AppEx: Sized
{
    fn app(self) -> Command;

    fn connection_options(self) -> Command
    {
        self.app()
            .cert_options(true)
            .arg(
                Arg::new("listen")
                    .short('l')
                    .value_name("port")
                    .required(true)
                    .help("Specify listening port")
                    .long_help(
                        "\
Specify the port that Proxide will listen to. This is the port that clients must comment to, either
directly (when using -t) or through a proxy setting (when using -p).",
                    ),
            )
            .arg(
                Arg::new("target")
                    .short('t')
                    .long("target")
                    .value_name("host:port")
                    .help("Enable direct connections to target server")
                    .long_help(
                        "\
Specify the target server to allow clients to connect directly to Proxide. Proxide will redirect
these connections to the target server.

Proxide will rewrite the HTTP Host-header and the HTTP/2 authority information with the correct
details for the target server. If the client embeds the target details in the actual message
payloads, these are not modified and the target server will receive the details client used to
connect to Proxide instead.",
                    ),
            )
            .arg(
                Arg::new("allow-remote")
                    .action(ArgAction::SetTrue)
                    .long("allow-remote")
                    .help("Allow remote connections")
                    .long_help(
                        "\
Allow Proxide to handle remote connections. Normally Proxide listens to only local connections.
Allowing remote connections allows running Proxide on a remote machine (such as the server
machine).

WARNING: It is recommended to use the '-t' option together with '--allow-remote'. Acting as a
CONNECT proxy ('-p') while allowing remote connections turns Proxide into an open proxy, which may
be used to connect to other hosts in the local network.",
                    ),
            )
            .arg(
                Arg::new("proxy")
                    .short('p')
                    .long("proxy")
                    .value_name("filter")
                    .num_args(0..=1)
                    .help("Enable CONNECT proxy behaviour.")
                    .long_help(
                        "\
Enable using Proxide as a CONNECT proxy with an optional filter for limiting the connections which
are decoded by Proxide.

The option accepts a comma separated list of filters to limit the decoded connections. Asterisk
('*') can be used as a wildcard when specifying the hostname filter. If the filter is not
specified, Proxide will decode all traffic.

  > proxide monitor -l 1234 -p *.foo.com,api.bar.com:8080

If neither -t or -p options are specified, Proxide will default to running as a CONNECT proxy (as
if -p was specified with no filter). If -t option is specified, this default behaviour is skipped.
Specify both -t and -p options if Proxide should act as a CONNECT proxy while redirecting direct
connections to a target server.
",
                    ),
            )
    }

    fn cert_options(self, connection: bool) -> Command
    {
        // Specify the common parts of the arguments.
        let cert = Arg::new("ca-certificate")
            .long("ca-cert")
            .value_name("path")
            .help(
                "Specify the CA certificate path. Defaults to 'proxide_ca.crt' if not specified.",
            );
        let key = Arg::new("ca-key").long("ca-key").value_name("path").help(
            "Specify the CA private key path. Defaults to 'proxide_ca.key' if not specified.",
        );

        // Specify everything specific to either connection or configuration usage.
        let (cert, key) = match connection {
            false => (cert, key),
            true => (
                cert.long_help(
                    "\
Specify the CA certificate path. Defaults to 'proxide_ca.crt' if not specified.

The CA certificate is used to produce temporary certificates for incoming TLS connections. This is
required for intercepting TLS traffic from the clients. For the TLS interception to succeed, the
clients must trust certificates signed by the specified CA certificate. A new CA certificate can be
created with 'proxide config ca --create'.",
                ),
                key.long_help(
                    "\
Specify the CA private key path. Defaults to 'proxide_ca.key' if not specified.

The CA private key is required to be able to use the CA certificate for signing generated
certificates.",
                ),
            ),
        };
        let app = self.app().arg(cert).arg(key);
        match connection {
            false => app,
            true => app.arg(
                Arg::new("no-ca")
                    .action(ArgAction::SetTrue)
                    .long("no-ca")
                    .conflicts_with_all(["ca-certificate", "ca-key"])
                    .help("Run without a CA certificate. TLS connections cannot be intercepted.")
                    .long_help(
                        "\
Run without a CA certificate. Proxide will reject any TLS connections, as it cannot generate
certificates for them. Without this option Proxide refuses to start if the CA certificate or
private key is missing.",
                    ),
            ),
        }
    }

    fn json_options(self) -> Command
    {
        self.app().arg(
            Arg::new("json")
                .long("json")
                .action(ArgAction::SetTrue)
                .help(
                "Output in JSON format. Disables UI when used with 'view' or 'monitor' commands.",
            ),
        )
    }

    fn decoder_options(self) -> Command
    {
        crate::decoders::setup_args(self.app())
    }
}

impl AppEx for Command
{
    fn app(self) -> Command
    {
        self
    }
}

#[cfg(test)]
mod test
{
    use super::*;
    use clap::error::ErrorKind;
    use clap::ArgMatches;

    fn parse(args: &[&str]) -> Result<ArgMatches, clap::Error>
    {
        setup_app("test")
            .try_get_matches_from(std::iter::once("proxide").chain(args.iter().copied()))
    }

    fn sub<'a>(m: &'a ArgMatches, name: &str) -> &'a ArgMatches
    {
        m.subcommand_matches(name).unwrap()
    }

    fn string<'a>(m: &'a ArgMatches, id: &str) -> Option<&'a str>
    {
        m.get_one::<String>(id).map(String::as_str)
    }

    fn error_kind(args: &[&str]) -> ErrorKind
    {
        parse(args).unwrap_err().kind()
    }

    #[test]
    fn command_definition_is_valid()
    {
        setup_app("test").debug_assert();
    }

    #[test]
    fn subcommand_is_required()
    {
        assert_eq!(
            error_kind(&[]),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
        assert_eq!(
            error_kind(&["config"]),
            ErrorKind::DisplayHelpOnMissingArgumentOrSubcommand
        );
        assert_eq!(error_kind(&["frobnicate"]), ErrorKind::InvalidSubcommand);
    }

    #[test]
    fn capture_options()
    {
        let m = parse(&[
            "capture",
            "-l",
            "1234",
            "-t",
            "host:443",
            "-f",
            "out.bin",
            "--json",
            "--allow-remote",
            "--ca-cert",
            "a.crt",
            "--ca-key",
            "a.key",
        ])
        .unwrap();
        let m = sub(&m, "capture");
        assert_eq!(string(m, "listen"), Some("1234"));
        assert_eq!(string(m, "target"), Some("host:443"));
        assert_eq!(string(m, "file"), Some("out.bin"));
        assert_eq!(string(m, "ca-certificate"), Some("a.crt"));
        assert_eq!(string(m, "ca-key"), Some("a.key"));
        assert!(m.get_flag("json"));
        assert!(m.get_flag("allow-remote"));
        assert!(!m.get_flag("no-ca"));
        assert_eq!(string(m, "proxy"), None);
    }

    #[test]
    fn flags_default_to_false()
    {
        let m = parse(&["monitor", "-l", "1"]).unwrap();
        let m = sub(&m, "monitor");
        assert!(!m.get_flag("json"));
        assert!(!m.get_flag("allow-remote"));
        assert!(!m.get_flag("no-ca"));
        assert!(!m.contains_id("proxy"));
        assert_eq!(string(m, "ca-certificate"), None);
    }

    #[test]
    fn listen_port_is_required()
    {
        assert_eq!(error_kind(&["capture"]), ErrorKind::MissingRequiredArgument);
    }

    #[test]
    fn no_ca_conflicts_with_ca_paths()
    {
        assert_eq!(
            error_kind(&["capture", "-l", "1", "--no-ca", "--ca-cert", "a.crt"]),
            ErrorKind::ArgumentConflict
        );
        let m = parse(&["capture", "-l", "1", "--no-ca"]).unwrap();
        assert!(sub(&m, "capture").get_flag("no-ca"));
    }

    #[test]
    fn proxy_filter_is_optional()
    {
        let m = parse(&["monitor", "-l", "1", "-p"]).unwrap();
        let m = sub(&m, "monitor");
        assert!(m.contains_id("proxy"));
        assert_eq!(string(m, "proxy"), None);

        let m = parse(&["monitor", "-l", "1", "-p", "*.foo.com,bar:1", "--no-ca"]).unwrap();
        assert_eq!(string(sub(&m, "monitor"), "proxy"), Some("*.foo.com,bar:1"));
    }

    #[test]
    fn grpc_accepts_several_values_and_repeats()
    {
        // `--grpc dir/*.proto` expanded by the shell gives several values for one occurrence.
        let m = parse(&[
            "view", "f.bin", "--grpc", "a.proto", "b.proto", "--grpc", "c.proto",
        ])
        .unwrap();
        let m = sub(&m, "view");
        assert_eq!(string(m, "file"), Some("f.bin"));
        let protos: Vec<_> = m
            .get_many::<String>("grpc")
            .unwrap()
            .map(String::as_str)
            .collect();
        assert_eq!(protos, ["a.proto", "b.proto", "c.proto"]);
    }

    #[test]
    fn view_requires_file()
    {
        assert_eq!(error_kind(&["view"]), ErrorKind::MissingRequiredArgument);
        let m = parse(&["view", "--json", "f.bin"]).unwrap();
        let m = sub(&m, "view");
        assert_eq!(string(m, "file"), Some("f.bin"));
        assert!(m.get_flag("json"));
    }

    #[test]
    fn log_options_are_global()
    {
        let m = parse(&["view", "f.bin", "--log", "l.txt", "--log-level", "info"]).unwrap();
        assert_eq!(string(&m, "log"), Some("l.txt"));
        assert_eq!(string(&m, "log-level"), Some("info"));

        let m = parse(&["--log", "l.txt", "view", "f.bin"]).unwrap();
        assert_eq!(string(&m, "log"), Some("l.txt"));
        assert_eq!(string(&m, "log-level"), Some("debug"));
    }

    #[test]
    fn log_level_is_validated_and_requires_log()
    {
        assert_eq!(
            error_kind(&["--log", "l.txt", "--log-level", "loud", "view", "f.bin"]),
            ErrorKind::InvalidValue
        );
        assert_eq!(
            error_kind(&["--log-level", "info", "view", "f.bin"]),
            ErrorKind::MissingRequiredArgument
        );
        // The default log level alone doesn't require --log.
        let m = parse(&["view", "f.bin"]).unwrap();
        assert_eq!(string(&m, "log"), None);
    }

    #[test]
    fn ca_requires_an_action()
    {
        assert_eq!(
            error_kind(&["config", "ca"]),
            ErrorKind::MissingRequiredArgument
        );
        assert_eq!(
            error_kind(&["config", "ca", "--force"]),
            ErrorKind::MissingRequiredArgument
        );
        assert_eq!(
            error_kind(&["config", "ca", "--duration", "5"]),
            ErrorKind::MissingRequiredArgument
        );
    }

    #[test]
    fn ca_create_options()
    {
        let m = parse(&["config", "ca", "--create"]).unwrap();
        let m = sub(sub(&m, "config"), "ca");
        assert!(m.get_flag("create"));
        assert!(!m.get_flag("force"));
        assert_eq!(m.get_one::<u32>("duration"), Some(&7));
        assert!(!m.contains_id("revoke"));
        assert!(!m.contains_id("trust"));

        let m = parse(&["config", "ca", "--create", "-f", "--duration", "30"]).unwrap();
        let m = sub(sub(&m, "config"), "ca");
        assert!(m.get_flag("force"));
        assert_eq!(m.get_one::<u32>("duration"), Some(&30));

        assert_eq!(
            error_kind(&["config", "ca", "--create", "--duration", "abc"]),
            ErrorKind::ValueValidation
        );
    }

    #[test]
    fn ca_store_value_is_optional()
    {
        let m = parse(&["config", "ca", "--revoke"]).unwrap();
        let m = sub(sub(&m, "config"), "ca");
        assert!(m.contains_id("revoke"));
        assert_eq!(string(m, "revoke"), None);
        assert!(!m.get_flag("create"));
        assert_eq!(m.get_one::<u32>("duration"), None);

        let m = parse(&["config", "ca", "--trust", "system", "--create"]).unwrap();
        let m = sub(sub(&m, "config"), "ca");
        assert_eq!(string(m, "trust"), Some("system"));
        assert!(m.get_flag("create"));

        assert_eq!(
            error_kind(&["config", "ca", "--trust", "--revoke"]),
            ErrorKind::ArgumentConflict
        );
    }
}
