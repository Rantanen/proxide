using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Google.Protobuf;
using Grpc.Core;

namespace DotNetGrpc
{
    class Program : HelloWorld.HelloWorldBase
    {
        static void Main( string[] args )
        {
            var clientPort = 8888;
            var serverPort = 8890;
            if( args.Length > 0 )
                serverPort = clientPort = int.Parse( args[ 0 ] );
            if( args.Length > 1 )
                serverPort = int.Parse( args[ 1 ] );

            Server server = new Server
            {
                Services =
                {
                    HelloWorld.BindService( new Program() ),
                },
                Ports =
                {
                    new ServerPort( "localhost", serverPort, ServerCredentials.Insecure )
                }
            };

            server.Start();
            Console.WriteLine( $"Listening on port {serverPort}" );

            Channel channel = new Channel( $"127.0.0.1:{clientPort}", ChannelCredentials.Insecure );
            var client = new HelloWorld.HelloWorldClient( channel );
            DoCalls( client ).Wait();

            channel.ShutdownAsync().Wait();
            server.ShutdownAsync().Wait();
        }

        public static async Task DoCalls(HelloWorld.HelloWorldClient client)
        {
            // var multiHelloStream = client.SayMultipleHello();
            for( int i = 0; i < 50; i++ )
            {
                var response = await client.SayHelloAsync( new HelloRequest
                    {Name = $"World{i}!", Data = ByteString.CopyFromUtf8( new string('x', 1000 )), B = true });
                Console.WriteLine( $"Received '{response.Message}'" );
                // await Task.Delay( 500 );
                // await multiHelloStream.RequestStream.WriteAsync( new HelloRequest
                // {
                    // Name = "Async " + i
                // } );
                // await multiHelloStream.ResponseStream.MoveNext();
                // Console.WriteLine( $"Received '{multiHelloStream.ResponseStream.Current.Message}' from stream" );
            }

            // await multiHelloStream.RequestStream.CompleteAsync();
        }

        public override async Task< HelloResponse > SayHello( HelloRequest request, ServerCallContext context )
        {
            return new HelloResponse
            {
                Message = $"Hello {request.Name} {request.B}",
            };
        }
    }
}
