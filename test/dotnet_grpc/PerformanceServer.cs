using System;
using System.Threading.Tasks;
using System.IO;
using Grpc.Core;
using System.Threading;
using Google.Protobuf;

using DotNet.Performance;

namespace dotnet_grpc
{
    class PerformanceServer
    {
        Server server;

        public PerformanceServer(Arguments args)
        {
            var credentials = args.GetServerCredentials();

            server = new Server
            {
                Services = { PerformanceService.BindService(new PerformanceServiceImpl()) },
                Ports = { new ServerPort("localhost", args.ServerPort, credentials) },
            };
            server.Start();
            Console.WriteLine($"C# Performance Server running in port {args.ServerPort}");
            Console.WriteLine($" - TLS: {credentials != ServerCredentials.Insecure}");
        }

        public async Task Stop()
        {
            Console.Write("Stopping server...");
            await server.ShutdownAsync();
            Console.WriteLine(" Done.");
        }

        class PerformanceServiceImpl : PerformanceService.PerformanceServiceBase
        {
            override public Task<PingMessage> Ping(PingMessage ping, ServerCallContext ctx)
            {
                return Task.FromResult(ping);
            }

            override public async Task<PingMessage> DelayedPing(PingMessage ping, ServerCallContext ctx)
            {
                await Task.Delay(10);
                return ping;
            }

            override public async Task PingStream(IAsyncStreamReader<PingMessage> requests, IServerStreamWriter<PingMessage> responses, ServerCallContext ctx)
            {
                while (await requests.MoveNext())
                {
                    await responses.WriteAsync(requests.Current);
                }
            }

            override public async Task<PingMessage> PingClientStream(IAsyncStreamReader<PingMessage> requests, ServerCallContext ctx)
            {
                long ul = 0;
                while (await requests.MoveNext())
                {
                    ul += requests.Current.Ticks;
                }
                return new PingMessage { Ticks = ul };
            }

            override public async Task PingServerStream(PingMessage request, IServerStreamWriter<PingMessage> responses, ServerCallContext ctx)
            {
                for (long i = 0; i < request.Ticks; i++)
                    await responses.WriteAsync(new PingMessage { Ticks = DateTime.Now.Ticks });
            }

            override public Task<PingMessage> SetBlob(Blob request, ServerCallContext ctx)
            {
                var response = new PingMessage();
                foreach (var item in request.Items) {
                    foreach (var sub in item.SubItems) {
                        foreach (var data in sub.Data) {
                            response.Ticks += data.Length;
                        }
                    }
                }
                return Task.FromResult(response);
            }

            override public Task<Blob> GetBlob(PingMessage request, ServerCallContext ctx)
            {
                var response = new Blob(request.Ticks);
                return Task.FromResult(response);
            }

            override public async Task GetSetBlob(IAsyncStreamReader<Blob> request, IServerStreamWriter<Blob> respone, ServerCallContext ctx)
            {
                await respone.WriteAsync(new Blob(30));
                await request.MoveNext();
            }
        }
    }
}
