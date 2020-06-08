using Grpc.Core;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Threading.Tasks;
using System;

using DotNet.Service;

namespace dotnet_grpc
{
    class GrpcClient
    {
        public async static Task Run(Arguments args)
        {
            var credentials = args.GetChannelCredentials();

            var options = new List<ChannelOption>();

            // Require gRPC 1.30 to actually work
            if (!String.IsNullOrEmpty(args.Proxy))
                options.Add(new ChannelOption("grpc.http_proxy", args.Proxy));

            Console.WriteLine($"C# Test Client connecting to {args.Connect}");
            var channel = new Channel(args.Connect, credentials, options);
            var client = new TestService.TestServiceClient(channel);

            var response = await client.HelloWorldAsync(new HelloWorldRequest {
                Name = "World!"
            });

            Trace.Assert(response.Hello.Greeting == "Hello");
            Trace.Assert(response.Hello.Name == "World!");

            Console.WriteLine("Tests OK");
        }
    }
}
