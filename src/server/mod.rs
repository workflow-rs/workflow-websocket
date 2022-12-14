//!
//! async WebSocket server functionality (requires tokio executor)
//! 
use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::net::SocketAddr;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::mpsc::*;
use tokio::net::{TcpListener, TcpStream};
use tungstenite::Message;
use tungstenite::Error as WebSocketError;
use tokio_tungstenite::{accept_async, WebSocketStream};
use futures_util::{SinkExt, StreamExt};
use workflow_log::*;

mod error;
mod result;

pub use error::Error;
pub use result::Result;

/// WebSocketHandler trait that represents the WebSocket processor
/// functionality.  This trait is supplied to the WebSocket
/// which subsequently invokes it's functions during websocket
/// connection and messages.  The trait can override `with_handshake()` method
/// to enable invocation of the `handshake()` method upon receipt of the
/// first valid websocket message from the incoming connection.
#[async_trait]
pub trait WebSocketHandler 
where 
    Arc<Self> : Sync
{
    /// Context type used by impl trait to represent websocket connection
    type Context : Send + Sync;
    /// Enables optional invocation of the handshake method with the first
    /// message received form the incoming websocket
    fn with_handshake(self : &Arc<Self>) -> bool { false }
    /// Sets the default connection timeout if no messages have been received
    fn with_timeout(self : &Arc<Self>) -> Duration { Duration::from_secs(3) }
    /// Called immediately when connection is established
    /// This function can return an error to terminate the connection
    async fn connect(self : &Arc<Self>, peer: SocketAddr) -> Result<Self::Context>;
    /// Called upon websocket disconnection
    async fn disconnect(self : &Arc<Self>, _ctx : Self::Context, _result : Result<()>) { }
    /// Called upon receipt of the first websocket message if `with_handshake()` returns true
    /// This function can return an error to terminate the connection
    async fn handshake(self : &Arc<Self>, _ctx : &Self::Context, _msg : Message, _sink : &UnboundedSender<tungstenite::Message>) -> Result<()> {  Ok(()) }
    /// Called for every websocket message
    /// This function can return an error to terminate the connection
    async fn message(self : &Arc<Self>, ctx : &Self::Context, msg : Message, sink : &UnboundedSender<tungstenite::Message>) -> Result<()>;
}

/// WebSocketServer that provides the main websocket connection
/// and message processing loop that delivers messages to the 
/// installed WebSocketHandler trait.
pub struct WebSocketServer<T> 
where T : WebSocketHandler + Send + Sync + 'static + Sized
{
    pub connections : AtomicU64,
    pub handler : Arc<T>,
}

impl<T> WebSocketServer<T>
where T : WebSocketHandler + Send + Sync + 'static
{
    pub fn new(handler : Arc<T>) -> Arc<Self> {
        let connections = AtomicU64::new(0);
        Arc::new(WebSocketServer {
            connections,
            handler,
        })
    }

    async fn accept_connection(self : &Arc<Self>, peer: SocketAddr, stream: TcpStream) {
        if let Err(e) = self.handle_connection(peer, stream).await {
            match e {
                Error::WebSocketError(WebSocketError::ConnectionClosed) 
                    | Error::WebSocketError(WebSocketError::Protocol(_))
                    | Error::WebSocketError(WebSocketError::Utf8) => (),
                err => log_error!("Error processing connection: {}", err),
            }
        }
    }

    async fn handle_connection(self: &Arc<Self>, peer: SocketAddr, stream: TcpStream) -> Result<()> {
        let ws_stream = accept_async(stream).await?;
        let mut ctx = self.handler.connect(peer).await?;
        log_trace!("WebSocket connected: {}", peer);
        
        let result = self.connection_task(&mut ctx, ws_stream).await;
        self.handler.disconnect(ctx, result).await;
        log_trace!("WebSocket disconnected: {}", peer);
        
        Ok(())
    }

    async fn connection_task(self: &Arc<Self>, ctx : &mut T::Context, ws_stream: WebSocketStream<TcpStream>) -> Result<()> {
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();
        let (sink, mut sink_receiver) = tokio::sync::mpsc::unbounded_channel::<tungstenite::Message>();
        
        let timeout_duration = self.handler.with_timeout();
        let delay = tokio::time::sleep(timeout_duration);
        if self.handler.with_handshake() {
            tokio::select! {
                msg = ws_receiver.next() => {
                    match msg {
                        Some(Ok(msg)) => {
                            self.handler.handshake(&ctx,msg,&sink).await?;
                        },
                        _ => {
                            return Err(Error::MalformedHandshake);
                        }
                    }
                }
                _ = delay => {
                    return Err(Error::ConnectionTimeout);
                }
            }
        }

        // let mut interval = tokio::time::interval(Duration::from_millis(1000));
        loop {
            tokio::select! {
                msg = sink_receiver.recv() => {
                    ws_sender.send(msg.unwrap()).await?;
                },
                msg = ws_receiver.next() => {
                    match msg {
                        Some(msg) => {
                            let msg = msg?;
                            match msg {
                                Message::Binary(_) | Message::Text(_)  => {
                                    self.handler.message(&ctx, msg, &sink).await?;
                                },
                                Message::Close(_) => {
                                    self.handler.message(&ctx, msg, &sink).await?;
                                    // log_trace!("gracefully closing connection");
                                    break;
                                },
                                _ => {
                                    // TODO - should we respond to Message::Ping(_) ?
                                }
                            }
                        }
                        None => {
                            return Err(Error::ConnectionClosed);
                        }
                    }
                }
                // _ = interval.tick() => {
                //     ws_sender.send(Message::Text("tick".to_owned())).await?;
                // }
            }
        }
        
        Ok(())
    }

    pub async fn listen(self : &Arc<Self>, addr : &str) -> Result<()> {
        let listener = TcpListener::bind(&addr)
            .await
            .expect(&format!("WebSocket server unable to listen on: {}", addr));
        // log_trace!("WebSocket server listening on: {}", addr);
    
        while let Ok((stream, _)) = listener.accept().await {
            let peer = stream.peer_addr()
                .expect("WebSocket connected streams should have a peer address");
            // log_trace!("Peer address: {}", peer);
    
            let self_ = self.clone();
            tokio::spawn(async move {
                self_.accept_connection(peer, stream).await;
            });
        }
        Ok(())
    }
}