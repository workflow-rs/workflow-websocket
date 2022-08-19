#[cfg(target_arch = "wasm32")]
mod wasm;
use manual_future::ManualFuture;
#[cfg(target_arch = "wasm32")]
pub use wasm::WebSocketInterface;

#[cfg(not(target_arch = "wasm32"))]
mod native;
#[cfg(not(target_arch = "wasm32"))]
pub use native::WebSocketInterface;

pub mod error;
pub mod message;
pub mod event;
pub mod state;
pub mod settings;

pub use error::*;
pub use message::*;
pub use settings::*;

pub use message::Message;
pub use settings::Settings;

use std::sync::Arc;
use async_std::channel::{Receiver,Sender,unbounded};
use message::DispatchMessage;

#[derive(Clone)]
pub struct WebSocket {
    client : Arc<WebSocketInterface>,
    pub dispatcher_tx : Sender<DispatchMessage>,
    pub strategy : DispatchStrategy,
    pub receiver_rx : Receiver<Message>,
    pub receiver_tx : Sender<Message>,
}

impl WebSocket {
    pub fn new(url : &str, settings : Settings) -> Result<WebSocket,Error> {

        let (receiver_tx, receiver_rx) = unbounded::<Message>();
        let (dispatcher_tx, dispatcher_tx_rx) = match settings.strategy {
            DispatchStrategy::Post | DispatchStrategy::Ack => { 
                let tx_rx = unbounded::<DispatchMessage>();
                let tx = tx_rx.0.clone();
                (tx, tx_rx)
            },
        };

        let client = Arc::new(WebSocketInterface::new(
            url,
            receiver_tx.clone(),
            dispatcher_tx_rx
        )?);

        // client.connect()?;

        let websocket = WebSocket {
            client,
            dispatcher_tx,
            strategy : settings.strategy,
            receiver_rx,
            receiver_tx,
        };

        Ok(websocket)
    }

    pub async fn connect(self : &Arc<Self>) -> Result<(), Error> {
        Ok(self.client.connect().await?)
    }

    // pub fn connect_as_task(self : &Arc<Self>) -> Result<(), Error> {
    //     let self_ = self.clone();
    //     crate::task::spawn(async move {
    //         self_.client.connect().await.ok();
    //     });
    //     Ok(())
    // }

    pub async fn disconnect(self : &Arc<Self>) -> Result<(), Error> {
        Ok(self.client.disconnect().await?)
    }

    // pub fn disconnect_as_task(self : &Arc<Self>) -> Result<(), Error> {
    //     let self_ = self.clone();
    //     crate::task::spawn(async move {
    //         self_.client.disconnect().await.ok();
    //     });
    //     Ok(())
    // }

    pub async fn send(&self, message: Message) -> std::result::Result<(),Error> {
        if !self.client.is_open() {
            return Err(Error::NotConnected);
        }
        
        match self.strategy {
            DispatchStrategy::Post => {
                self.dispatcher_tx.send(DispatchMessage::Post(message)).await?;
            },
            DispatchStrategy::Ack => {
                let (future, ack) = ManualFuture::<()>::new();
                self.dispatcher_tx.send(DispatchMessage::WithAck(message, ack)).await?;
                future.await;
            }
        }
        Ok(())
    }


    // pub async fn try_send(&self, message: Message) -> std::result::Result<(),Error> {
    //     if !self.client.is_open() {
    //         return Err(Error::NotConnected);
    //     }
        
    //     match self.strategy {
    //         DispatchStrategy::Post => {
    //             self.dispatcher_tx.try_send(DispatchMessage::Post(message))?;
    //         },
    //         DispatchStrategy::Ack => {
    //             #[cfg(target_arch = "wasm32")]
    //             panic!("try_send() is not supported with DispatcherStrategy::Ack");
    //             #[cfg(not(target_arch = "wasm32"))]
    //             async_std::task::block_on(async move {
    //                 let (future, ack) = ManualFuture::<()>::new();
    //                 self.dispatcher_tx.send(DispatchMessage::WithAck(message, ack)).await.ok();
    //                 future.await;
    //             });
    //         }
    //     }

    //     Ok(())
    // }

}
