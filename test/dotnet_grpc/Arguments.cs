using CommandLine;

namespace dotnet_grpc
{
    class Arguments
    {
        [Option(
            "connect",
            Required = true,
            HelpText = "Server address the client connects to")]
        public string Connect { get; set; }

        [Option(
            "server-port",
            Required = true,
            HelpText = "Port in which the gRPC server is hosted")]
        public int ServerPort { get; set; }

        [Option(
            "ca-cert",
            HelpText = "TLS CA certificate used to valide the server. " +
                "Enables the client to connect using TLS. ('default' to use " +
                "the default system certificates.)")]
        public string CACertificate { get; set; }

        [Option(
            "server-cert",
            HelpText = "TLS certificate used to enable TLS server. " +
                "Server is hosted using insecure channel if certficate " +
                "is not provided")]
        public string ServerCertificate { get; set; }

        [Option(
            "server-key",
            HelpText = "TLS private key used to enable TLS server. " +
                "Must be supplied if server-cert is used.")]
        public string ServerPrivateKey { get; set; }

        [Option(
            "proxy",
            HelpText = "CONNECT proxy to use with the connection.")]
        public string Proxy { get; set; }
    }
}
