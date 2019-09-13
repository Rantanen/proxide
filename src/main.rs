
use std::error::Error;
use h2::{server, client, RecvStream, SendStream};
use http::{Request, Response, StatusCode};
use tokio::net::{TcpListener, TcpStream};

async fn pipe_stream( source : &mut RecvStream, target : &mut SendStream<bytes::Bytes> ) -> Result<bytes::Bytes, Box<dyn Error>> {

    let mut chunks = bytes::Bytes::new();
    // target.send_data( bytes::Bytes::from_static( b"\0" ), false )?;
    while let Some( data ) = source.data().await {
        let b = data?;
        chunks.extend_from_slice( &b );
        target.send_data( b, source.is_end_stream() )?;
    }
    let trailers = source.trailers().await?;
    if let Some( trailers ) = trailers {
        target.send_trailers( trailers )?;
    }

    Ok(chunks)
}

pub async fn handle_socket( socket : TcpStream ) -> Result<(), Box<dyn Error>> {

    println!( "New socket" );
    let mut client_h2 = server::handshake( socket ).await?;
    println!( "Handshaked" );
    let server_tcp = TcpStream::connect( "127.0.0.1:8890" ).await?;
    println!( "TCP connected" );
    let (server_h2, server_conn) = client::handshake(server_tcp).await?;
    println!( "Server connected" );

    tokio::spawn( async move {
        match server_conn.await {
            Ok(..) => {},
            Err( e ) => eprintln!( "Error: {:?}", e )
        }
    });

    let mut server_h2 = server_h2.ready().await?;
    while let Some( request ) = client_h2.accept().await {

        let (request, mut respond ) = match request {
            Ok( r ) => r,
            Err( e ) => {
                println!( "Connection error: {:?}", e );
                return Ok(())
            }
        };

        println!( "Received {:?}", request );
        let (request, mut request_body) = request.into_parts();
        let request = Request::from_parts( request, () );

        println!( "Relaying request" );
        let (response, mut server_stream) = server_h2.send_request( request, false ).unwrap();
        println!( "Relaying body" );
        let request_data = pipe_stream( &mut request_body, &mut server_stream ).await?;
        println!( "Body: {:?}", request_data );

        let response = response.await?;
        println!( "Received response {:?}", response );
        let (response, mut response_body) = response.into_parts();
        let response = Response::from_parts( response, () );

        println!( "Relaying response" );
        let mut client_stream = respond.send_response( response, false )?;
        println!( "Relaying body" );
        let response_data = pipe_stream( &mut response_body, &mut client_stream ).await?;

    }
    Ok(())
}

#[tokio::main]
pub async fn main()-> Result<(), Box<dyn Error>> {

    env_logger::init();
    log::info!("Test");

    let mut listener = TcpListener::bind( "0.0.0.0:8888" ).await.unwrap();

    loop {

        if let Ok( (socket, src_addr) ) = listener.accept().await {

            println!( "New connection from {:?}", src_addr );
            tokio::spawn( async {

                match handle_socket( socket ).await {
                    Ok(..) => {}
                    Err( e ) => println!( "Error {:?}", e ),
                }
            } );
        }
    }
}
