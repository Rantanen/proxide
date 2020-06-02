using System.IO;
using System;
using System.Threading.Tasks;
using System.Diagnostics;
using Grpc.Core;

using DotNet.Service;

namespace dotnet_grpc
{
    class GrpcClient
    {
        public async static Task Run(Arguments args)
        {
            ChannelCredentials credentials;
            if (string.IsNullOrEmpty(args.CACertificate))
            {
                credentials = ChannelCredentials.Insecure;
            }
            else
            {
                var cert = File.ReadAllText(args.CACertificate);
                credentials = new SslCredentials(cert);
            }

            Console.WriteLine($"C# Test Client connecting to {args.Connect}");
            var channel = new Channel(args.Connect, credentials);
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
