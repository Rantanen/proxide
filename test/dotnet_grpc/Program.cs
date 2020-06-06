using System.Threading.Tasks;
using System.Threading;
using System;
using System.Collections.Generic;
using CommandLine;

namespace dotnet_grpc
{
    class Program
    {
        static object StatusLock = new object();
        static bool Running = true;

        static void Main(string[] rawArgs)
        {
            CommandLine.Parser.Default.ParseArguments<Arguments>(rawArgs)
                .WithParsed(args => Run(args).Wait())
                .WithNotParsed(OnError);
        }

        async static Task Run(Arguments args)
        {
            SetupExitHandlers();

            var server = new GrpcServer(args);

            await GrpcClient.Run(args);

            await server.Stop();
        }

        static void OnError(IEnumerable<Error> errs)
        {
            Environment.ExitCode = 1;
        }

        static void SetupExitHandlers()
        {
            // Handler for SIGTERM.
            AppDomain.CurrentDomain.ProcessExit += (s, e) => {
                lock(StatusLock) {
                    Running = false;
                    Monitor.PulseAll(StatusLock);
                }
            };

            // Handler for Ctrl-C.
            Console.CancelKeyPress += (s, e) => {
                e.Cancel = true;
                lock(StatusLock) {
                    Running = false;
                    Monitor.PulseAll(StatusLock);
                }
            };
        }

        static void WaitForExit()
        {
            // Wait for process exit.
            lock(StatusLock) {
                while (Running)
                    Monitor.Wait(StatusLock);
            }
        }
    }
}
