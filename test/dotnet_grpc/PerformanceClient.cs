using Grpc.Core;
using System.Collections.Generic;
using System.Diagnostics;
using System.IO;
using System.Threading.Tasks;
using System;

using DotNet.Performance;

namespace dotnet_grpc
{
    class PerformanceClient
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
            var client = new PerformanceService.PerformanceServiceClient(channel);

            Console.Write("1000 serial ping... ");
            var sw = Stopwatch.StartNew();
            for (int i = 0; i < 1000; i++)
            {
                await client.PingAsync(new PingMessage {
                    Ticks = DateTime.Now.Ticks
                });
            }
            Console.Write(sw.ElapsedMilliseconds);
            Console.WriteLine(" ms");

            Console.Write("10000 parallel ping... ");
            var pings = new List<Task>();
            sw.Restart();
            for (int i = 0; i < 10000; i++)
            {
                pings.Add(client.PingAsync(new PingMessage {
                    Ticks = DateTime.Now.Ticks
                }).ResponseAsync);
            }
            Task.WaitAll(pings.ToArray());
            Console.Write(sw.ElapsedMilliseconds);
            Console.WriteLine(" ms");

            Console.Write("10000 parallel delayed ping... ");
            pings = new List<Task>();
            sw.Restart();
            for (int i = 0; i < 10000; i++)
            {
                pings.Add(client.DelayedPingAsync(new PingMessage {
                    Ticks = DateTime.Now.Ticks
                }).ResponseAsync);
            }
            Task.WaitAll(pings.ToArray());
            Console.Write(sw.ElapsedMilliseconds);
            Console.WriteLine(" ms");

            Console.Write("1000 ping stream... ");
            sw.Restart();
            var readerWriter = client.PingStream();
            for (int i = 0; i < 1000; i++)
            {
                await readerWriter.RequestStream.WriteAsync(new PingMessage { Ticks = DateTime.Now.Ticks });
                await readerWriter.ResponseStream.MoveNext();
            }
            readerWriter.Dispose();
            Console.Write(sw.ElapsedMilliseconds);
            Console.WriteLine(" ms");

            Console.Write("10000 client ping stream... ");
            sw.Restart();
            var writer = client.PingClientStream();
            for (int i = 0; i < 10000; i++)
            {
                await writer.RequestStream.WriteAsync(new PingMessage { Ticks = DateTime.Now.Ticks });
            }
            await writer.RequestStream.CompleteAsync();
            await writer.ResponseAsync;
            Console.Write(sw.ElapsedMilliseconds);
            Console.WriteLine(" ms");

            Console.Write("10000 server ping stream... ");
            sw.Restart();
            var reader = client.PingServerStream(new PingMessage { Ticks = 10000 });
            int count = 0;
            while (await reader.ResponseStream.MoveNext())
                count++;
            Console.Write(sw.ElapsedMilliseconds);
            Console.WriteLine(" ms");
            Debug.Assert(count == 10000);

            Console.Write("Get Blob... ");
            sw.Restart();
            var blob = await client.GetBlobAsync(new PingMessage { Ticks = 30 });
            Console.Write(sw.ElapsedMilliseconds);
            Console.WriteLine(" ms");

            Console.Write("Set Blob... ");
            sw.Restart();
            var response = await client.SetBlobAsync(new Blob(30));
            Console.Write(sw.ElapsedMilliseconds);
            Console.WriteLine(" ms");

            Console.Write("Get/Set Blob... ");
            sw.Restart();
            var blobRW = client.GetSetBlob();
            await blobRW.RequestStream.WriteAsync(new Blob(30));
            await blobRW.ResponseStream.MoveNext();
            Console.Write(sw.ElapsedMilliseconds);
            Console.WriteLine(" ms");

            Console.WriteLine("Tests OK");
        }
    }
}

/*
service PerformanceService
{
    rpc Ping(PingMessage) returns (PingMessage);
    rpc DelayedPing(PingMessage) returns (PingMessage);

    rpc PingStream(stream PingMessage) returns (stream PingMessage);
    rpc PingClientStream(stream PingMessage) returns (PingMessage);
    rpc PingServerStream(PingMessage) returns (stream PingMessage);

    rpc SetBlob(Blob) returns (PingMessage);
    rpc GetBlob(PingMessage) returns (Blob);
}
*/
