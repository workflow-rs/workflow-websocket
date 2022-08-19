// use std::fmt::Debug;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::AtomicU64;
use std::net::SocketAddr;
use std::time::Duration;
use ahash::AHashMap;
use async_trait::async_trait;
use tokio::sync::mpsc::*;
use tokio::net::{TcpListener, TcpStream};
use tungstenite::{Message, Result};
use tokio_tungstenite::{accept_async, tungstenite::Error};
use futures_util::{SinkExt, StreamExt};
use workflow_log::*;

#[async_trait]
pub trait WebSocketHandler {
    async fn handle_message(self : &Arc<Self>, msg : Message, sink : UnboundedSender<tungstenite::Message>);
}

pub struct WebSocketServer<T> 
where T : WebSocketHandler + Send + Sync + 'static + Sized
{
    pub connections : AtomicU64,
    pub handler : Arc<T>,

    handlers : Arc<Mutex<AHashMap<String, Arc<T>>>>,
}

impl<T> WebSocketServer<T>
where T : WebSocketHandler + Send + Sync + 'static
{
    // pub fn new(handler : Box<&'handler dyn WebSocketHandler>) -> Arc<Self> {
    pub fn new(handler : Arc<T>) -> Arc<Self> {
        let connections = AtomicU64::new(0);
        let handlers = Arc::new(Mutex::new(AHashMap::new()));
        Arc::new(WebSocketServer {
            connections,
            handler,
            handlers,

        })
    }

    pub fn register_handler(self : &Arc<Self>, name : &str, handler : Arc<T>) {
        self.handlers.lock().unwrap().insert(name.to_string(), handler);
    }
        
    async fn accept_connection(self : &Arc<Self>, peer: SocketAddr, stream: TcpStream) {
        if let Err(e) = self.handle_connection(peer, stream).await {
            match e {
                Error::ConnectionClosed | Error::Protocol(_) | Error::Utf8 => (),
                err => log_error!("Error processing connection: {}", err),
            }
        }
    }

    async fn handle_connection(self: &Arc<Self>, peer: SocketAddr, stream: TcpStream) -> Result<()> {
        let self_ = self.clone();
        let ws_stream = accept_async(stream).await.expect("Failed to accept");
        log_trace!("New WebSocket connection: {}", peer);
        let (mut ws_sender, mut ws_receiver) = ws_stream.split();
        // let (mut ws_sender, mut ws_receiver) = ws_stream.split();
        let mut interval = tokio::time::interval(Duration::from_millis(1000));

        // let ws_sender = Arc::new(Mutex::new(ws_sender));
        // Echo incoming WebSocket messages and send a message periodically every second.

        // TODO - create local request tracking to execute async functions?


        // let mut state = HandlerBinding::Init;
        let delay = tokio::time::sleep(Duration::from_millis(3000));
        let handler : Option<Arc<T>>;// = None;

        tokio::select! {
            msg = ws_receiver.next() => {
                match msg {
                    Some(Ok(Message::Text(text))) if text.starts_with("WORKFLOW-RPC") => {
                        let parts = text.split(':').collect::<Vec<&str>>();
                        if parts.len() == 3 {
                            handler = self_.handlers.lock().unwrap().get(&parts[2].to_string()).cloned();
                        } else {
                            log_trace!("invalid header, terminating..");
                            return Ok(());
                        }
                    },
                    _ => {
                        log_trace!("invalid header, terminating..");
                        return Ok(());
                    }
                }
            }
            _ = delay => {
                log_trace!("connection timed out before header");
                return Ok(());
                // ws_sender.send(Message::Text("tick".to_owned())).await?;
            }
        }

        if handler.is_none() {
            log_trace!("invalid header, terminating..");
            return Ok(());
        }

        let handler = handler.unwrap();

        // let msg = ws_receiver.next().await;
        // if let Some(Ok(Message::Text(text))) = msg {
        //     if text.starts_with("WORKFLOW-RPC") {

        //     } else {
        //         return Ok(());
        //     }
        //     let vv = "WORKFLOW-RPC:"
        // } else {
        //     return Ok(());
        // }
        // let msg = start.
        // if let Ok(start) = start {

        // }

        let (sink, mut sink_receiver) = tokio::sync::mpsc::unbounded_channel::<tungstenite::Message>();

        // let self_ = self.clone();

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
                                        handler.handle_message(msg, sink.clone()).await;//.expect("error handling request");
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
                _ = interval.tick() => {
                    ws_sender.send(Message::Text("tick".to_owned())).await?;
                }
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