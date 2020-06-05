use clap::{App, AppSettings, Arg, ArgGroup, SubCommand};

pub fn setup_app() -> App<'static, 'static>
{
    App::new("Proxide - HTTP2 debugging proxy")
        .version(env!("CARGO_PKG_VERSION"))
        .author("Mikko Rantanen <rantanen@jubjubnest.net>")
        .setting(AppSettings::SubcommandRequiredElseHelp)
        .subcommand(
            SubCommand::with_name("view")
                .about("View traffic from a session or capture file")
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
                .connection_options()
                .json_options()
                .decoder_options(),
        )
        // Capture subcommand.
        .subcommand(
            SubCommand::with_name("capture")
                .about("Capture network traffic into a file for later analysis")
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
                // The "config ca" subcommand.
                .subcommand(
                    SubCommand::with_name("ca")
                        .about("Manage CA certificates required for debugging TLS traffic")
                        .cert_options()
                        .arg(
                            Arg::with_name("create")
                                .long("create")
                                .help("Create a new CA certificate"),
                        )
                        .arg(
                            Arg::with_name("force")
                                .short("f")
                                .long("force")
                                .help("Overwrite existing files")
                                .requires("create"),
                        )
                        .arg(
                            Arg::with_name("revoke")
                                .long("revoke")
                                .takes_value(true)
                                .min_values(0)
                                .help(
                                    "Revokes existing Proxide CA certificates \
                                   from the trusted CA certificate store",
                                )
                                .possible_values(&["user", "system", "all"]),
                        )
                        .arg(
                            Arg::with_name("trust")
                                .long("trust")
                                .takes_value(true)
                                .min_values(0)
                                .help(
                                    "Imports the current Proxide CA certificate \
                                   to the CA certificate store",
                                )
                                .possible_values(&["user", "system", "all"]),
                        )
                        .group(
                            ArgGroup::with_name("action")
                                .args(&["create", "revoke", "trust"])
                                .multiple(true)
                                .required(true),
                        )
                        .arg(
                            Arg::with_name("duration")
                                .long("duration")
                                .help(
                                    "Specifies the number of days the new CA certificate is valid",
                                )
                                .default_value_if("create", None, "7")
                                .requires("create")
                                .validator(|v| {
                                    v.parse::<u32>()
                                        .map_err(|_| {
                                            String::from("duration must be a positive number")
                                        })
                                        .map(|_| ())
                                }),
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
            .cert_options()
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

    fn cert_options(self) -> App<'a, 'b>
    {
        self.app()
            .arg(
                Arg::with_name("ca-certificate")
                    .long("ca-cert")
                    .value_name("path")
                    .default_value("proxide_ca.crt")
                    .help("Specify the CA certificate used by Proxide to sign the generated TLS certificates"),
            )
            .arg(
                Arg::with_name("ca-key")
                    .long("ca-key")
                    .value_name("path")
                    .default_value("proxide_ca.key")
                    .help("Specify the CA private key used by Proxide to sign the generated TLS certificates"),
            )
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
