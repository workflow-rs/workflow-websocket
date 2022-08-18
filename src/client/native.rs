pub use workflow_core as core;
pub use workflow_log::*;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{WebSocketStream, MaybeTlsStream};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use async_std::channel::{Receiver,Sender};
use super::message::{Message,DispatchMessage,Ctl};
use super::error::Error;

struct Settings {
    url : String,
}

#[allow(dead_code)]
struct Inner {
    ws_stream: Option<WebSocketStream<MaybeTlsStream<TcpStream>>>,
}

pub struct WebSocketInterface {
    inner : Arc<Mutex<Option<Inner>>>,
    settings : Arc<Mutex<Settings>>,
    // reconnect : Arc<Mutex<bool>>,
    reconnect : AtomicBool,
    is_open : AtomicBool,
    receiver_tx : Sender<Message>,
    dispatcher_tx_rx : (Sender<DispatchMessage>,Receiver<DispatchMessage>),
}

impl WebSocketInterface {

    pub fn new(
        url : &str, 
        receiver_tx : Sender<Message>,
        dispatcher_tx_rx : (Sender<DispatchMessage>,Receiver<DispatchMessage>),
    ) -> Result<WebSocketInterface,Error> {
        let settings = Settings { 
            url: url.to_string()
        };

        let iface = WebSocketInterface {
            inner : Arc::new(Mutex::new(None)),
            settings : Arc::new(Mutex::new(settings)),
            receiver_tx,
            dispatcher_tx_rx,
            reconnect : AtomicBool::new(true),
            is_open : AtomicBool::new(false),
        };

        Ok(iface)
    }

    pub fn url(self : &Arc<Self>) -> String {
        self.settings.lock().unwrap().url.clone()
    }

    pub fn is_open(self : &Arc<Self>) -> bool {
        self.is_open.load(Ordering::SeqCst)
    }

    pub async fn connect(self : &Arc<Self>) -> Result<(),Error> {
        let self_ = self.clone();
        
        log_trace!("connect...");
        if self_.inner.lock().unwrap().is_some() {
            return Err(Error::AlreadyConnected);
        }

        let (connect_tx, connect_rx) = tokio::sync::oneshot::channel();
        let mut connect_tx = Some(connect_tx);

        self_.reconnect.store(true, Ordering::SeqCst);
        core::task::spawn(async move {
            
            loop {

                match connect_async(&self_.url()).await {
                    Ok(stream) => {

                        self_.is_open.store(true, Ordering::SeqCst);
                        let (ws_stream,_) = stream;

                        *self_.inner.lock().unwrap() = Some(Inner {
                            ws_stream : Some(ws_stream)
                        });
                
                        // connect_completer.lock().unwrap().take().unwrap().complete(()).await;
                        connect_tx.take().unwrap().send(()).unwrap();

                        if let Err(err) = self_.dispatcher().await {
                            log_error!("{}",err);
                        }

                        self_.is_open.store(false, Ordering::SeqCst);
                    },
                    Err(e) => {
                        log_error!("failed to connect: {}", e);
                        // continue;
                    }
                };

                if self_.reconnect.load(Ordering::SeqCst) == false {
                    break;
                };
            }
        });

        // connect_future.await;
        connect_rx.await.map_err(|_| Error::ConnectChannel)?;

        Ok(())
    }

    async fn dispatcher(self: &Arc<Self>) -> Result<(), Error> {

        let (mut ws_sender, mut ws_receiver) = self.inner.lock().unwrap().as_mut().unwrap().ws_stream.take().unwrap().split();
        let (_, dispatcher_rx) = &self.dispatcher_tx_rx;
        loop {
            tokio::select! {
                dispatch = dispatcher_rx.recv() => {

                    match dispatch.unwrap() {
                        DispatchMessage::Post(message) => {
                            match message {
                                Message::Binary(data) => {
                                    ws_sender.send(data.into()).await?;
                                },
                                Message::Text(text) => {
                                    ws_sender.send(text.into()).await?;
                                },
                                Message::Ctl(_) => {
                                    panic!("WebSocket Error: dispatcher received unexpected Ctl message")
                                }
                            }
                        },
                        
                        DispatchMessage::WithAck(message,completer) => {
                            match message {
                                Message::Binary(data) => {
                                    ws_sender.send(data.into()).await?;
                                    completer.complete(()).await;
                                },
                                Message::Text(text) => {
                                    ws_sender.send(text.into()).await?;
                                    completer.complete(()).await;
                                },
                                Message::Ctl(_) => {
                                    panic!("WebSocket Error: dispatcher received unexpected Ctl message")
                                }
                            }
                        },
                        DispatchMessage::DispatcherShutdown => {
                            log_trace!("WebSocket - local close");
                            ws_sender.close().await?;
                            break;
                        }
                    }
                },
                msg = ws_receiver.next() => {
                    match msg {
                        Some(msg) => {
                            match msg {
                                Ok(msg) => {
                                    match msg {
                                        WsMessage::Binary(data) => {
                                            self
                                                .receiver_tx
                                                .send(Message::Binary(data))
                                                .await
                                                .map_err(|_|Error::ReceiveChannel)?;
                                        },
                                        WsMessage::Text(text) => {
                                            self
                                                .receiver_tx
                                                .send(Message::Text(text))
                                                .await
                                                .map_err(|_|Error::ReceiveChannel)?;
                                        },
                                        WsMessage::Close(_) => {
                                            log_trace!("WebSocket: gracefully closed connection");
                                            self.receiver_tx.send(Message::Ctl(Ctl::Closed)).await?;
                                            break;
                                        },
                                        WsMessage::Ping(_) => { },
                                        WsMessage::Pong(_) => { },
                                        WsMessage::Frame(_frame) => { },
                                    }
                                },
                                Err(e) => {
                                    log_error!("websocket error: {}", e);
                                    break;
                                }
                            }
                        },
                        None => {
                            log_error!("channel closed...");
                            break;
                        }
                    }
                }
            }
        }
    
        *self.inner.lock().unwrap() = None;

        Ok(())
    }

    async fn close(self : &Arc<Self>) -> Result<(),Error> {
        if self.inner.lock().unwrap().is_some() {
            self.dispatcher_tx_rx.0.send(DispatchMessage::DispatcherShutdown)
                .await.ok();
            *self.inner.lock().unwrap() = None;
        } else {
            log_error!("Error: disconnecting from non-initialized connection");
        }

        Ok(())
    }

    // async fn reconnect(self : &Arc<Self>) -> Result<(),Error> {
    //     log_trace!("... starting reconnect");

    //     self.close().await?;
    //     self.connect().await?;

    //     Ok(())
    // }

    pub async fn disconnect(self : &Arc<Self>) -> Result<(),Error> {
        self.reconnect.store(false, Ordering::SeqCst);
        self.close().await?;
        Ok(())
    }
}

impl Drop for WebSocketInterface {
    fn drop(&mut self) {

    }
}