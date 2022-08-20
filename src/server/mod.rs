use std::sync::Arc;
use std::sync::atomic::AtomicU64;
use std::net::SocketAddr;
use std::time::Duration;
use async_trait::async_trait;
use tokio::sync::mpsc::*;
use tokio::net::{TcpListener, TcpStream};
use tungstenite::Message;
use tungstenite::Error as WebSocketError;
use tokio_tungstenite::accept_async;
use futures_util::{SinkExt, StreamExt};
use workflow_log::*;
use thiserror::Error;

pub type Result<T> = std::result::Result<T, Error>;

#[derive(Debug, Error)]
pub enum Error {
    #[error("Connection timeout")]
    ConnectionTimeout,

    #[error("Malformed handshake message")]
    MalformedHandshake,

    #[error("Negotiation failure")]
    NegotiationFailure,

    #[error("Negotiation failure: {0}")]
    NegotiationFailureWithReason(String),

    #[error("WebSocket error: {0}")]
    WebSocketError(#[from] tungstenite::Error),

}

#[async_trait]
pub trait WebSocketHandler {
    type Context : Send + Sync;
    fn with_handshake(self : &Arc<Self>) -> bool { false }
    fn with_timeout(self : &Arc<Self>) -> Duration { Duration::from_secs(3) }
    async fn connect(self : &Arc<Self>, peer: SocketAddr) -> Result<Self::Context>;
    async fn handshake(self : &Arc<Self>, ctx : &Self::Context, msg : Message, sink : &UnboundedSender<tungstenite::Message>) -> Result<()>;
    async fn message(self : &Arc<Self>, ctx : &Self::Context, msg : Message, sink : &UnboundedSender<tungstenite::Message>) -> Result<()>;
}

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
        let ctx = self.handler.connect(peer).await?;
        log_trace!("New WebSocket connection: {}", peer);
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
                            match msg {
                                Ok(msg) => {
                                    if msg.is_text() ||msg.is_binary() {
                                        self.handler.message(&ctx, msg, &sink).await?;
                                    } else if msg.is_close() {
                                        log_trace!("gracefully closing connection");
                                        break;
                                    }
                                },
                                Err(err) => {
                                    log_error!("Closing connection: {}", err);
                                    break;
                                }
                            }
                        }
                        None => break,
                    }
                }
                // _ = interval.tick() => {
                //     ws_sender.send(Message::Text("tick".to_owned())).await?;
                // }
            }

            log_trace!("LOOP TASK FINISHED SELECT!");
        }
        
        log_trace!("LOOP TASK FINISHED LOOP!");
        Ok(())
    }

    pub async fn listen(self : &Arc<Self>, addr : &str) -> Result<()> {
        let listener = TcpListener::bind(&addr).await.expect("Can't listen");
        log_trace!("Listening on: {}", addr);
    
        while let Ok((stream, _)) = listener.accept().await {
            let peer = stream.peer_addr().expect("connected streams should have a peer address");
            log_trace!("Peer address: {}", peer);
    
            let self_ = self.clone();
            tokio::spawn(async move {
                self_.accept_connection(peer, stream).await;
            });
        }
        Ok(())
    }
}