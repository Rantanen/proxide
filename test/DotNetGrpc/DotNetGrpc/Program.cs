using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Google.Protobuf;
using Grpc.Core;
using Test;

namespace DotNetGrpc
{
    class Program : HelloWorld.HelloWorldBase
    {
        static void Main( string[] args )
        {
            var clientPort = 5555;
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
            var response = await client.SayHelloAsync( new HelloRequest {Name = $"World"} );
            Console.ReadKey();

            var complexStream = client.ComplexTypesStream();
            var stream = complexStream.RequestStream;
            _ = Task.Run( async () =>
            {
                while( await complexStream.ResponseStream.MoveNext() ) ;
            } );
            await stream.WriteAsync( new ComplexTypeStream
            {
                SetValue = new ComplexType
                {
                    SingleString = "Foo"
                }
            });
            Console.ReadKey();
            await stream.WriteAsync( new ComplexTypeStream { GetValue = true });
            Console.ReadKey();
            await stream.WriteAsync( new ComplexTypeStream { GetValue = true });
            Console.ReadKey();
            await stream.WriteAsync( new ComplexTypeStream
            {
                SetValue = new ComplexType
                {
                    ManyStrings = {"Foo", "Bar", "Baz"},
                    Children =
                    {
                        new ChildType
                        {
                            Name = "Apple",
                            NameUtf16 = ByteString.CopyFrom( Encoding.Unicode.GetBytes( "Apple" ) )
                        },
                        new ChildType
                        {
                            Name = "Orange",
                            NameUtf16 = ByteString.CopyFrom( Encoding.Unicode.GetBytes( "Orange" ) )
                        }
                    },
                }
            } );
            Console.ReadKey();
            await stream.WriteAsync( new ComplexTypeStream { GetValue = true });
            Console.ReadKey();
            await stream.WriteAsync( new ComplexTypeStream { Close = true });
        }

        public override async Task< HelloResponse > SayHello( HelloRequest request, ServerCallContext context )
        {
            return new HelloResponse
            {
                Message = $"Hello {request.Name}!",
            };
        }
        public override async Task SayMultipleHello( IAsyncStreamReader< HelloRequest > requestStream, IServerStreamWriter< HelloResponse > responseStream,
            ServerCallContext context )
        {
            while( await requestStream.MoveNext())
            {
                await responseStream.WriteAsync( new HelloResponse
                {
                    Message = "Hellooooo " + requestStream.Current.Name
                } );
            }
        }

        public override async Task< ComplexType > ComplexTypes( ComplexType request, ServerCallContext context )
        {
            return request;
        }

        public override async Task ComplexTypesStream( IAsyncStreamReader< ComplexTypeStream > requestStream, IServerStreamWriter< ComplexType > responseStream,
            ServerCallContext context )
        {
            ComplexType stored = null;
            while( await requestStream.MoveNext() )
            {
                if( requestStream.Current.Close )
                    return;

                if( requestStream.Current.GetValue )
                    await responseStream.WriteAsync( stored );
                else if( requestStream.Current.SetValue != null )
                    stored = requestStream.Current.SetValue;
            }
        }
    }
}
