pub use workflow_core as core;
pub use workflow_log::*;
use futures_util::{SinkExt, StreamExt};
use tokio::net::TcpStream;
use tokio_tungstenite::{WebSocketStream, MaybeTlsStream};
use tokio_tungstenite::{connect_async, tungstenite::protocol::Message as WsMessage};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use super::message::{Message,DispatchMessage,Ctl};
use super::error::Error;
use workflow_core::channel::*;

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
    sender_tx_rx : (Sender<DispatchMessage>,Receiver<DispatchMessage>),
}

impl WebSocketInterface {

    pub fn new(
        url : &str, 
        receiver_tx : Sender<Message>,
        sender_tx_rx : (Sender<DispatchMessage>,Receiver<DispatchMessage>),
    ) -> Result<WebSocketInterface,Error> {
        let settings = Settings { 
            url: url.to_string()
        };

        let iface = WebSocketInterface {
            inner : Arc::new(Mutex::new(None)),
            settings : Arc::new(Mutex::new(settings)),
            receiver_tx,
            sender_tx_rx,
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

    pub async fn connect(self : &Arc<Self>, block : bool) -> Result<(),Error> {
        let self_ = self.clone();
        
        if self_.inner.lock().unwrap().is_some() {
            return Err(Error::AlreadyConnected);
        }

        let (connect_trigger, connect_listener) = triggered::trigger();
        let mut connect_trigger = Some(connect_trigger);

        self_.reconnect.store(true, Ordering::SeqCst);

        core::task::spawn(async move {
            
            let mut seq = 0;
            loop {
                seq += 1;
                println!("STARTING LOOP... {}", seq);
                match connect_async(&self_.url()).await {
                    Ok(stream) => {

                        log_trace!("connected...");

                        self_.is_open.store(true, Ordering::SeqCst);
                        let (ws_stream,_) = stream;

                        *self_.inner.lock().unwrap() = Some(Inner {
                            ws_stream : Some(ws_stream)
                        });
                
                        if connect_trigger.is_some() {
                            connect_trigger.take().unwrap().trigger();
                        }

                        if let Err(err) = self_.dispatcher().await {
                            log_error!("dispatcher: {}",err);
                        }

                        self_.is_open.store(false, Ordering::SeqCst);
                    },
                    Err(e) => {
                        log_error!("failed to connect: {}", e);
                        workflow_core::task::sleep(Duration::from_millis(1000)).await;
                    }
                };

                if self_.reconnect.load(Ordering::SeqCst) == false {
                    break;
                };
            }
        });

        if block {
            connect_listener.await;
        }

        Ok(())
    }

    async fn dispatcher(self: &Arc<Self>) -> Result<(), Error> {

        let (mut ws_sender, mut ws_receiver) = self.inner.lock().unwrap().as_mut().unwrap().ws_stream.take().unwrap().split();
        let (_, sender_rx) = &self.sender_tx_rx;

        self.receiver_tx.send(Message::Ctl(Ctl::Open)).await?;

        loop {
            tokio::select! {
                dispatch = sender_rx.recv() => {
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
                        
                        DispatchMessage::WithAck(message,ack_sender) => {
                            match message {
                                Message::Binary(data) => {
                                    let result = ws_sender.send(data.into()).await
                                        .map(|ok|Arc::new(ok.into()))
                                        .map_err(|err|Arc::new(err.into()));
                                    ack_sender.send(result).await?;
                                },
                                Message::Text(text) => {
                                    let result = ws_sender.send(text.into()).await
                                        .map(|ok|Arc::new(ok.into()))
                                        .map_err(|err|Arc::new(err.into()));
                                    ack_sender.send(result.into()).await?;
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
                                    self.receiver_tx.send(Message::Ctl(Ctl::Closed)).await?;
                                    log_error!("websocket error: {}", e);
                                    break;
                                }
                            }
                        },
                        None => {
                            self.receiver_tx.send(Message::Ctl(Ctl::Closed)).await?;
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
            self.sender_tx_rx.0.send(DispatchMessage::DispatcherShutdown)
                .await.ok();
            *self.inner.lock().unwrap() = None;
        } else {
            log_error!("Error: disconnecting from non-initialized connection");
        }

        Ok(())
    }

    pub async fn disconnect(self : &Arc<Self>) -> Result<(),Error> {
        self.reconnect.store(false, Ordering::SeqCst);
        self.close().await?;
        Ok(())
    }
}
