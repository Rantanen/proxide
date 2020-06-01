using System;
using System.Collections.Generic;
using System.IO;
using System.Linq;
using System.Security.Cryptography.X509Certificates;
using System.Text;
using System.Threading;
using System.Threading.Tasks;
using Google.Protobuf;
using Grpc.Core;
using Test;
using Test.Package;

namespace DotNetGrpc
{
    class Program : HelloWorld.HelloWorldBase
    {
        private const string CERTIFICATE = @"
-----BEGIN CERTIFICATE-----
MIIDCTCCAfGgAwIBAgIUK/Rj+8wx5YNlMiHt0gXFIYGxX5cwDQYJKoZIhvcNAQEL
BQAwFDESMBAGA1UEAwwJbG9jYWxob3N0MB4XDTIwMDUzMDE5MDE0NVoXDTIxMDUz
MDE5MDE0NVowFDESMBAGA1UEAwwJbG9jYWxob3N0MIIBIjANBgkqhkiG9w0BAQEF
AAOCAQ8AMIIBCgKCAQEA4xgnMlIx4cV+6YFeJxn/rceb3peIisNuBt5F/4gZ9Ed7
8habJbStlPW8o9DEcX+AGh7hHY2/QFdx4CvxOhCqRd0G/WUgzNF6JRW/cYMGMjfL
cELcyGmHfKPXhkBJFoSX1Fs2zwYvdW0WgEiT1VHkARySGCBXGp4J/lADbQFGZDs9
QInwnd0HTGY5gQxErNTEP88LwxhcFyPI6SgQvyQT2dr3pH2Dw4130U29pik1jiLU
BW/gyTafTaoG8PyLmmtldg8/7U60aDjenf2S6mprryLRHKXFKp+/JPj9tqIzdQIn
ggpOLbfq2HOlv0bwEq7cDE5NzaJGsoKlP9qbRomAfQIDAQABo1MwUTAdBgNVHQ4E
FgQUfoQbvrMauD6gxfFdCRYSuAVeNXAwHwYDVR0jBBgwFoAUfoQbvrMauD6gxfFd
CRYSuAVeNXAwDwYDVR0TAQH/BAUwAwEB/zANBgkqhkiG9w0BAQsFAAOCAQEAL7bY
Fav0N4nYuEs3owjmKWmC6IFfBW9RT4atGj6QLPENHJtEOEcgNaTjrCfV3z1diIF3
CC+MATWRooQEHItbGTQg2wzR684SeFOM0I56jWqJ47IdK2/ikXYeBSGN6+c8HyHV
pnT0JbAwcYdj/H3o2emHQc9MQcf8p2JFgjVPH5ZAgXwIN9yQDKpKIB++fkyTIFm6
r7ZMYD3AZJCGCcaUwg4whGxk5tkJ4SgF6dY2kbV4o83KSvuhVq8N1w1LhlgqvgbG
BYo8DUErXmDdKbv2hRC0czZTBV5ZG9ItexhY4LqREOnHrGJcJAEFHe2FZ6T01ioz
8K4q5GG2wLjB33yZzw==
-----END CERTIFICATE-----
";

        private const string PRIVATE_KEY = @"
-----BEGIN PRIVATE KEY-----
MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQDjGCcyUjHhxX7p
gV4nGf+tx5vel4iKw24G3kX/iBn0R3vyFpsltK2U9byj0MRxf4AaHuEdjb9AV3Hg
K/E6EKpF3Qb9ZSDM0XolFb9xgwYyN8twQtzIaYd8o9eGQEkWhJfUWzbPBi91bRaA
SJPVUeQBHJIYIFcangn+UANtAUZkOz1AifCd3QdMZjmBDESs1MQ/zwvDGFwXI8jp
KBC/JBPZ2vekfYPDjXfRTb2mKTWOItQFb+DJNp9Nqgbw/Iuaa2V2Dz/tTrRoON6d
/ZLqamuvItEcpcUqn78k+P22ojN1AieCCk4tt+rYc6W/RvASrtwMTk3NokaygqU/
2ptGiYB9AgMBAAECggEBANi9k6NmaW9WxDLuosLlAG6GhVBkBhCSRy/M8rfY2RSZ
KUW7p6XMFKOICdu7g9HjY4sKj8ZaI/+vteLDbb8CatC1DFfKLgztnQtJ/2bCK6bA
M61YU0n/1izyXuAl+NvB/vrRd7UM7TzAueoD2vyM5PTepNzb+OZRka4kBbECt4Eu
cGJI2FY7W42ZYhw3sxjbyVuW2I6jqaEncp0T0q3wEZ5uiCtIbTEX+X577HTvFtFv
tzgIIjTxUyaVRj39cDaZ4kfjr2BetKvqNScXb78er3L+xE9vz4eVFv8FdzajzT7n
c015zoE3RWrdhUsfqtCT3CbevG2E1mx9iD9747OtvC0CgYEA+PH9Rh0L2FE2M8TI
woxsx3FJEDYq+f5qe+qNiiQhs5SfCIFWyBVOcytJugNqfTX8UONafi6uP5I1/AjG
Y1oVYpvOks01TuZa5srTahijkoCgig/Xyqy56f2DRD/Vyn1XIn5y2lF4tjnSFR8b
m3Lt8vuiXPSftuQmK3cvbAGKnaMCgYEA6Yekmisl57nTcuEnHpI9Tph07nOasgeP
/CfaYvmCPq7V8HbRWwXt8APN64aSdBc+qT8J4R2YD614d7Z9UmUF+y7/hJggzvKE
fb5tnhRo40izrizc6ciNRkhTwecoZms5o+aMeMFWdD/toxLFjBSSQwrKb1rWGyf3
Ayy5fxKRC18CgYAeT/jzDJ5gnKLo8tEvP0IPlu+6lZ3uCtiUdh797yBbaYFj27vh
aRbAV0kG6VuSG3y5rLVcH/r/qqIAKmFdv55S/33LykjvboUrDQ9pH87rC9aAeSVh
fF626zOMn+k8Wr69aIA7rSfxqGC4Sa1m5DutFo7SmsbH0kgDiuOvVxC12QKBgFjZ
GJDnNaayFnawnteMv/J1IpfON97f7bH736SkVR9QGWlBa2l8GgilCeU/79xnM5nk
t/eD8OSWFS1Gqut8MAhe2ywxTNovfqTwnHf2P+mpMWNlAi+X89f3kJZHQiGlTerD
vlH7DM9xuxG+BJbFBNio9FflcWwnil0U2QY1pCV1AoGAEwfiAOQQTqHfpHEIBFHF
sEF4va804Veg0N47D/j3oMR0KSXyM6hC5nFu/OVGyvKSGZNmaYuDKwxAAO1Feimp
qARZWZVQm11uH68OMHXFFmMte4RCBAP8IYFzPN/jPjk9bdrb7+Fjr4NfYrsvbEKk
YIASF0UIVSsCfbWh6FOcIHk=
-----END PRIVATE KEY-----
";

        static void Main( string[] args )
        {
            Environment.SetEnvironmentVariable( "http_proxy", "http://127.0.0.1:5555" );
            Environment.SetEnvironmentVariable( "https_proxy", "http://127.0.0.1:5555" );
            // Environment.SetEnvironmentVariable( "GRPC_TRACE", "all,-api" );
            // Environment.SetEnvironmentVariable( "GRPC_VERBOSITY", "DEBUG" );

            bool secure = false;

            var clientPort = 8890;
            var serverPort = 8890;
            if( args.Length > 0 )
                serverPort = clientPort = int.Parse( args[ 0 ] );
            if( args.Length > 1 )
                serverPort = int.Parse( args[ 1 ] );

            var serverCredentials = secure
                ? new SslServerCredentials( new[] {new KeyCertificatePair( CERTIFICATE, PRIVATE_KEY ),} )
                : ServerCredentials.Insecure;
            var clientCredentials = secure
                ? new SslCredentials( GetRootCerts() )
                : ChannelCredentials.Insecure;

            Server server = new Server
            {
                Services =
                {
                    HelloWorld.BindService( new Program() ),
                },
                Ports =
                {
                    new ServerPort( "localhost", serverPort, serverCredentials )
                }
            };

            server.Start();
            Console.WriteLine( $"Listening on port {serverPort}" );

            Channel channel = new Channel( $"localhost:{clientPort}", clientCredentials );
            var client = new HelloWorld.HelloWorldClient( channel );

            DoCalls( client ).Wait();

            channel.ShutdownAsync().Wait();
            server.ShutdownAsync().Wait();
        }

        public static async Task DoCalls(HelloWorld.HelloWorldClient client)
        {
            var response = await client.SayHelloAsync( new HelloRequest {Name = $"World"} );

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
            await stream.WriteAsync( new ComplexTypeStream { GetValue = true });
            await stream.WriteAsync( new ComplexTypeStream { GetValue = true });
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
            await stream.WriteAsync( new ComplexTypeStream { GetValue = true });
            await stream.WriteAsync( new ComplexTypeStream { Close = true });
        }

        public override async Task< HelloResponse > SayHello( HelloRequest request, ServerCallContext context )
        {
            Console.WriteLine("Serving SayHello");
            return new HelloResponse
            {
                Message = $"Hello {request.Name}!",
            };
        }
        public override async Task SayMultipleHello( IAsyncStreamReader< HelloRequest > requestStream, IServerStreamWriter< HelloResponse > responseStream,
            ServerCallContext context )
        {
            Console.WriteLine("New SayMultipleHello");
            while( await requestStream.MoveNext())
            {
                Console.WriteLine("Serving SayMultipleHello");
                await responseStream.WriteAsync( new HelloResponse
                {
                    Message = "Hellooooo " + requestStream.Current.Name
                } );
            }
        }

        public override async Task< ComplexType > ComplexTypes( ComplexType request, ServerCallContext context )
        {
            Console.WriteLine("Serving ComplexTypes");
            return request;
        }

        public override async Task ComplexTypesStream( IAsyncStreamReader< ComplexTypeStream > requestStream, IServerStreamWriter< ComplexType > responseStream,
            ServerCallContext context )
        {
            Console.WriteLine( "New ComplexTypesStream" );
            ComplexType stored = null;
            while( await requestStream.MoveNext() )
            {
                Console.WriteLine("Serving ComplexTypesStream: " + requestStream.Current.ValueCase);
                if( requestStream.Current.Close )
                    return;

                if( requestStream.Current.GetValue )
                    await responseStream.WriteAsync( stored );
                else if( requestStream.Current.SetValue != null )
                    stored = requestStream.Current.SetValue;
            }
        }

        public static string GetRootCerts()
        {
            X509Store store = new X509Store(StoreName.Root);
            store.Open( OpenFlags.ReadOnly );

            StringBuilder builder = new StringBuilder();
            foreach( X509Certificate2 cert in store.Certificates )
            {
                builder.AppendLine( "-----BEGIN CERTIFICATE-----" );
                builder.AppendLine( Convert.ToBase64String( cert.Export( X509ContentType.Cert ),
                    Base64FormattingOptions.InsertLineBreaks ) );
                builder.AppendLine( "-----END CERTIFICATE-----" );
            }

            return builder.ToString();
        }
    }
}
