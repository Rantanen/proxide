using System.Threading.Tasks;
using System.Threading;
using System;
using System.Collections.Generic;
using CommandLine;

namespace dotnet_grpc
{
    class Program
    {
        static void Main(string[] rawArgs)
        {
            CommandLine.Parser.Default.ParseArguments<Arguments>(rawArgs)
                .WithParsed(args => Run(args).Wait())
                .WithNotParsed(OnError);
        }

        async static Task Run(Arguments args)
        {
            switch (args.TestSuite)
            {
                case TestSuite.Basic:
                {
                    var server = new GrpcServer(args);
                    await GrpcClient.Run(args);
                    await server.Stop();
                    break;
                }
                case TestSuite.Performance:
                {
                    var server = new PerformanceServer(args);
                    await PerformanceClient.Run(args);
                    await server.Stop();
                    break;
                }
            }
        }

        static void OnError(IEnumerable<Error> errs)
        {
            Environment.ExitCode = 1;
        }
    }
}
