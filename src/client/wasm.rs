use std::sync::{Arc, Mutex};
use workflow_allocator::prelude::*;
use web_sys::{ErrorEvent as WsErrorEvent, MessageEvent as WsMessageEvent, WebSocket};
use js_sys::{ArrayBuffer,Uint8Array};
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;
use web_sys::{CloseEvent as WsCloseEvent};
// use workflow_allocator::error::{ error_code, Error, ErrorCode };
use async_std::channel::{Receiver,Sender};
// use workflow_allocator::result::Result;
use workflow_allocator::log::*;
use manual_future::{ManualFuture, ManualFutureCompleter};

use super::message::{Message,DispatchMessage,Ctl};
// TODO remove CloseEvent?
use super::event::CloseEvent;
// TODO remove State?
use super::state::State;
use super::error::Error;

// pub use super::message::Message;

impl TryFrom<u16> for State {
	type Error = Error;

	fn try_from(state: u16) -> std::result::Result<Self,Self::Error> {
		match state {
			WebSocket::CONNECTING => Ok(State::Connecting),
			WebSocket::OPEN => Ok(State::Open),
			WebSocket::CLOSING => Ok(State::Closing),
			WebSocket::CLOSED => Ok(State::Closed),
			_ => Err(Error::InvalidState(state)),
		}
	}
}

impl From<WsCloseEvent> for CloseEvent {
	fn from(event: WsCloseEvent) -> Self {
		Self {
			code: event.code(),
			reason: event.reason(),
			was_clean: event.was_clean(),
		}
	}
}

impl TryFrom< WsMessageEvent > for Message {
	type Error = Error;

	fn try_from(event: WsMessageEvent) -> std::result::Result<Self,Self::Error> {
		match event.data() {
			data if data.is_instance_of::<ArrayBuffer>() => {
				let buffer = Uint8Array::new(data.unchecked_ref());
				Ok( Message::Binary(buffer.to_vec()))
			},
			data if data.is_string() => {
				match data.as_string() {
					Some(text) => Ok(Message::Text(text)),
					None => Err(Error::DataEncoding),
				}
			},
			_ => Err(Error::DataType),
		}
	}
}

struct Settings {
    url : String,
}

#[allow(dead_code)]
struct Inner {

    ws : WebSocket,
    onmessage : Closure::<dyn FnMut(WsMessageEvent)>,
    onerror : Closure::<dyn FnMut(WsErrorEvent)>,
    onopen : Closure::<dyn FnMut()>,
    onclose : Closure::<dyn FnMut(WsCloseEvent)>,
    dispatcher_shutdown : Option<ManualFuture<()>>,
}

pub struct WebSocketInterface {
    inner : Arc<Mutex<Option<Inner>>>,
    settings : Arc<Mutex<Settings>>,
    reconnect : Arc<Mutex<bool>>,
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
            // reconnect: true,
            url: url.to_string()
        };

        let iface = WebSocketInterface {
            inner : Arc::new(Mutex::new(None)),
            settings : Arc::new(Mutex::new(settings)),
            receiver_tx,
            dispatcher_tx_rx,
            reconnect : Arc::new(Mutex::new(true)),
        };

        Ok(iface)
    }

    pub fn url(self : &Arc<Self>) -> String {
        self.settings.lock().unwrap().url.clone()
    }

    pub fn is_open(self : &Arc<Self>) -> bool {
        self.inner.lock().unwrap().as_ref().unwrap().ws.ready_state() == WebSocket::OPEN
    }

    pub async fn connect(self : &Arc<Self>) -> Result<(),Error> {
        
        log_trace!("connect...");
        let mut inner = self.inner.lock().unwrap();
        if inner.is_some() {
            return Err(Error::AlreadyConnected);
        }
        
        let (connect_future, connect_completer) = ManualFuture::<()>::new();
        let connect_completer = Arc::new(Mutex::new(Some(connect_completer)));

        *self.reconnect.lock().unwrap() = true;
        let receiver_tx = self.receiver_tx.clone();
        let ws = WebSocket::new(&self.url())?;
        ws.set_binary_type(web_sys::BinaryType::Arraybuffer);

        let receiver_tx_ = receiver_tx.clone();
        let onmessage = Closure::<dyn FnMut(_)>::new(move |event: WsMessageEvent| {
            let msg: Message = event.try_into().expect("MessageEvent Error");
            log_trace!("received message: {:?}", msg);
            receiver_tx_.try_send(msg).expect("WebSocket: Unable to send message via the receiver_tx channel");
        });
        ws.set_onmessage(Some(onmessage.as_ref().unchecked_ref()));
    
        let onerror = Closure::<dyn FnMut(_)>::new(move |event: WsErrorEvent| {
            log_trace!("error event: {:?}", event);
        });
        ws.set_onerror(Some(onerror.as_ref().unchecked_ref()));

        let receiver_tx_ = receiver_tx.clone();
        let onopen = Closure::<dyn FnMut()>::new(move || {
            receiver_tx_.try_send(Message::Ctl(Ctl::Open)).expect("WebSocket: Unable to send message via the receiver_tx channel");
            log_trace!("open event");
            if connect_completer.lock().unwrap().is_some() {
                let completer = connect_completer.lock().unwrap().take().unwrap();
                crate::task::spawn(async move {
                    completer.complete(()).await;
                });
            }
        });
        ws.set_onopen(Some(onopen.as_ref().unchecked_ref()));

        let receiver_tx_ = receiver_tx.clone();
        let ws_ = ws.clone();
        let self_ = self.clone();
        let onclose = Closure::<dyn FnMut(_)>::new(move |event : WsCloseEvent| {
            let event: CloseEvent = event.into();
            log_trace!("close event: {:?}", event);
            receiver_tx_.try_send(Message::Ctl(Ctl::Closed)).expect("WebSocket: Unable to send message via the receiver_tx channel");

            Self::cleanup(&ws_);
            let self_ = self_.clone();
            crate::task::spawn(async move {
                log_trace!("reconnecting...");
                self_.shutdown_dispatcher().await.expect("Unable to shutdown dispatcher");
                if *self_.reconnect.lock().unwrap() {
                    log_trace!("sleeping... 1 sec...");
                    async_std::task::sleep(std::time::Duration::from_millis(1000)).await;
                    self_.reconnect().await.ok();
                }
                // self_.reconnect().ok();
            });
        });
        ws.set_onclose(Some(onclose.as_ref().unchecked_ref()));

        let (dispatcher_shutdown, completer) = ManualFuture::<()>::new();
        self.dispatcher_task(ws.clone(),completer, self.dispatcher_tx_rx.1.clone());
        let dispatcher_shutdown = Some(dispatcher_shutdown);

        *inner = Some(Inner {
            ws,
            onmessage,
            onerror,
            onopen,
            onclose,
            dispatcher_shutdown,
        });

        connect_future.await;

        Ok(())
    
    }

    fn ws(self: &Arc<Self>) -> Result<WebSocket,Error> {
        Ok(self.inner.lock().unwrap().as_ref().unwrap().ws.clone())
    }

    pub fn try_send(self : &Arc<Self>, message : &Message) -> Result<(),Error> {

        let ws = self.ws()?;
        match message {
            Message::Binary(data) => {
                match ws.send_with_u8_array(&data) {
                    Ok(_) => log_trace!("binary message successfully sent"),
                    Err(err) => log_trace!("error sending message: {:?}", err),
                }
            },
            Message::Text(text) => {
                match ws.send_with_str(&text) {
                    Ok(_) => log_trace!("message successfully sent"),
                    Err(err) => log_trace!("error sending message: {:?}", err),
                }
            },
            _ => { }
        }

        Ok(())
    }

    fn dispatcher_task(
        self : &Arc<Self>,
        ws : WebSocket,
        completer : ManualFutureCompleter<()>,
        dispatcher_rx : Receiver<DispatchMessage>
    ) {
        crate::task::spawn(async move {

            let key = generate_random_pubkey();
            loop {
                let dispatch = dispatcher_rx.recv().await.unwrap();

                if ws.ready_state() != WebSocket::OPEN && !dispatch.is_ctl() {
                    log_error!("WebSocket Error: websocket is not connected");
                    continue;
                }

                match dispatch {
                    DispatchMessage::Post(message) => {
                        match message {
                            Message::Binary(data) => {
                                match ws.send_with_u8_array(&data) {
                                    Ok(_) => log_trace!("binary message successfully sent"),
                                    Err(err) => log_trace!("error sending message: {:?}", err),
                                }
                            },
                            Message::Text(text) => {
                                match ws.send_with_str(&text) {
                                    Ok(_) => log_trace!("message successfully sent"),
                                    Err(err) => log_trace!("error sending message: {:?}", err),
                                }
                            },
                            Message::Ctl(_) => {
                                panic!("WebSocket Error: dispatcher received unexpected Ctl message")
                            }
                        }
                    },
                    
                    DispatchMessage::WithAck(message,_completer) => {
                        match message {
                            Message::Binary(data) => {
                                // ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~                
                                // ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~                
                                // ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~                
                                let _result = ws.send_with_u8_array(&data);
                                //  {
                                    //     Ok(_) => log_trace!("binary message successfully sent"),
                                    //     Err(err) => log_trace!("error sending message: {:?}", err),
                                    // }
                                },
                                Message::Text(text) => {
                                // ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~                
                                // ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~                
                                // ~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~~                
                                let _result = ws.send_with_str(&text);
                                
                            },
                            Message::Ctl(_) => {
                                panic!("WebSocket Error: dispatcher received unexpected Ctl message")
                            }
                        }
                    },
                    DispatchMessage::DispatcherShutdown => {
                        break;
                    }
                }

                log_trace!("loop {}",key);
            }

            log_trace!("signaling SHUTDOWN...");
            completer.complete(()).await;

        });

    }

    fn cleanup(ws: &WebSocket) {
        ws.set_onopen(None);
        ws.set_onclose(None);
        ws.set_onerror(None);
        ws.set_onmessage(None);
    }

    async fn shutdown_dispatcher(self : &Arc<Self>) -> Result<(),Error> {
        // match &self.dispatcher_tx_rx {
        //     Some(tx_rx) => {
                self.dispatcher_tx_rx.0.send(DispatchMessage::DispatcherShutdown)
                .await
                .expect("WebSocket error: unable to dispatch ctl for dispatcher shutdown");
    
            let dispatcher = self.inner.lock().unwrap().as_mut().unwrap().dispatcher_shutdown.take().unwrap();
    
            log_trace!("!!!! waiting for dispatcher to shutdown...");
            dispatcher.await;
            log_trace!("!!!! dispatcher shutdown is done!");
    
            // },
            // None => { }
        // }

        Ok(())
    }

    async fn close(self : &Arc<Self>) -> Result<(),Error> {

        let mut inner = self.inner.lock().unwrap();
        if let Some(inner_) = &mut *inner {
            inner_.ws.close()?;
            *inner = None;
        } else {
            log_error!("Error: disconnecting from non-initialized connection");
        }

        Ok(())
    }
    async fn reconnect(self : &Arc<Self>) -> Result<(),Error> {
        log_trace!("... starting reconnect");

        self.close().await?;
        self.connect().await?;

        Ok(())
    }
    pub async fn disconnect(self : &Arc<Self>) -> Result<(),Error> {
        *self.reconnect.lock().unwrap() = false;
        
        self.close().await.ok();
        Ok(())
    }
}

impl Drop for WebSocketInterface {
    fn drop(&mut self) {

    }
}