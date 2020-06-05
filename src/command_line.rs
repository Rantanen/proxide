use clap::{App, AppSettings, Arg, ArgGroup, SubCommand};

macro_rules! long {
    ($doc:expr) => {
        concat!($doc, "\n ")
    };
}

pub fn setup_app() -> App<'static, 'static>
{
    App::new("Proxide - HTTP2 debugging proxy")
        .version(env!("CARGO_PKG_VERSION"))
        .author("Mikko Rantanen <rantanen@jubjubnest.net>")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .setting(AppSettings::UnifiedHelpMessage)
        .subcommand(
            SubCommand::with_name("view")
                .about("View traffic from a session or capture file")
                .setting(AppSettings::UnifiedHelpMessage)
                .json_options()
                .decoder_options()
                .arg(
                    Arg::with_name("file")
                        .short("f")
                        .value_name("file")
                        .required(true)
                        .help("Specify the file to load"),
                ),
        )
        // Monitor subcommand.
        .subcommand(
            SubCommand::with_name("monitor")
                .about("Monitor network traffic using the Proxide UI")
                .setting(AppSettings::UnifiedHelpMessage)
                .connection_options()
                .json_options()
                .decoder_options(),
        )
        // Capture subcommand.
        .subcommand(
            SubCommand::with_name("capture")
                .about("Capture network traffic into a file for later analysis")
                .setting(AppSettings::UnifiedHelpMessage)
                .connection_options()
                .json_options()
                .arg(
                    Arg::with_name("file")
                        .short("f")
                        .value_name("file")
                        .required(true)
                        .help("Specify the output file"),
                ),
        )
        // The config subcommands.
        .subcommand(
            SubCommand::with_name("config")
                .about("Manage Proxide configuration")
                .setting(AppSettings::UnifiedHelpMessage)
                // The "config ca" subcommand.
                .subcommand(
                    SubCommand::with_name("ca")
                        .about("Manage CA certificates required for debugging TLS traffic")
                        .setting(AppSettings::UnifiedHelpMessage)
                        .cert_options(false)
                        .arg(
                            Arg::with_name("create")
                                .long("create")
                                .help("Create a new CA certificate.")
                                .long_help(long!(
                                    "\
Creates a new CA certificate. Proxide will require a CA certificate for intercepting TLS traffic.
The CA certificate is used to sign certificates generated on the fly and must be trusted by the
clients for them to accept these certificates."
                                )),
                        )
                        .arg(
                            Arg::with_name("force")
                                .short("f")
                                .long("force")
                                .requires("create")
                                .help("Overwrite existing files.")
                                .long_help(long!(
                                    "\
Allows --create to overwrite existing files."
                                )),
                        )
                        .arg(
                            Arg::with_name("revoke")
                                .long("revoke")
                                .takes_value(true)
                                .min_values(0)
                                .value_name("store")
                                .help(
                                    "\
Revokes existing Proxide CA certificates from platform store",
                                )
                                .long_help(long!(
                                    "\
Revokes all existing Proxide CA certificates from the platform truted CA certificate store.

An optional value can be used to specify whether to revoke the certificates from the user-level or
system-level store a user-level or system-level store. Changes to the system-level store require
administrative privileges."
                                )),
                        )
                        .arg(
                            Arg::with_name("trust")
                                .long("trust")
                                .takes_value(true)
                                .min_values(0)
                                .value_name("store")
                                .conflicts_with("revoke")
                                .help(
                                    "\
Imports the current Proxide CA certificate to the platform store",
                                )
                                .long_help(long!(
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
command when it is not needed anymore."
                                )),
                        )
                        .arg(
                            Arg::with_name("duration")
                                .long("duration")
                                .default_value_if("create", None, "7")
                                .requires("create")
                                .validator(|v| {
                                    v.parse::<u32>()
                                        .map_err(|_| {
                                            String::from("duration must be a positive number")
                                        })
                                        .map(|_| ())
                                })
                                .help(
                                    "\
The number of days the new CA certificate is valid. Defaults to 7 days.",
                                )
                                .long_help(long!(
                                    "\
Specifies the number of days the new CA certificate is valid. Defaults to 7 days.

The ASN.1 format does not support arbitrary large dates so the user-specified value is
automatically capped to 2000 years."
                                )),
                        )
                        .group(
                            ArgGroup::with_name("action")
                                .args(&["create", "revoke", "trust"])
                                .multiple(true)
                                .required(true),
                        ),
                ),
        )
}

trait AppEx<'a, 'b>: Sized
{
    fn app(self) -> App<'a, 'b>;

    fn connection_options(self) -> App<'a, 'b>
    {
        self.app()
            .cert_options(true)
            .arg(
                Arg::with_name("listen")
                    .short("l")
                    .value_name("port")
                    .required(true)
                    .help("Specify listening port")
                    .takes_value(true),
            )
            .arg(
                Arg::with_name("target")
                    .short("t")
                    .value_name("host:port")
                    .required(true)
                    .help("Specify target host and port")
                    .takes_value(true),
            )
    }

    fn cert_options(self, connection: bool) -> App<'a, 'b>
    {
        // Specify the common parts of the arguments.
        let cert = Arg::with_name("ca-certificate")
            .long("ca-cert")
            .value_name("path")
            .help(
                "Specify the CA certificate path. Defaults to 'proxide_ca.crt' if not specified.",
            );
        let key = Arg::with_name("ca-key")
            .long("ca-key")
            .value_name("path")
            .help(
                "Specify the CA private key path. Defaults to 'proxide_ca.key' if not specified.",
            );

        // Specify everything specific to either connection or configuration usage.
        let (cert, key) = match connection {
            false => (cert, key),
            true => (
                cert.long_help(long!(
                    "\
Specify the CA certificate path. Defaults to 'proxide_ca.crt' if not specified.

The CA certificate is used to produce temporary certificates for incoming TLS connections. This is
required for intercepting TLS traffic from the clients. For the TLS interception to succeed, the
clients must trust certificates signed by the specified CA certificate."
                )),
                key.long_help(long!(
                    "\
Specify the CA private key path. Defaults to 'proxide_ca.key' if not specified.

The CA private key is required to be able to use the CA certificate for signing generated
certificates."
                )),
            ),
        };
        self.app().arg(cert).arg(key)
    }

    fn json_options(self) -> App<'a, 'b>
    {
        self.app().arg(Arg::with_name("json").long("json").help(
            "Output in JSON format. Disables UI when used with 'view' or 'monitor' commands.",
        ))
    }

    fn decoder_options(self) -> App<'a, 'b>
    {
        crate::decoders::setup_args(self.app())
    }
}

impl<'a, 'b> AppEx<'a, 'b> for App<'a, 'b>
{
    fn app(self) -> App<'a, 'b>
    {
        self
    }
}
