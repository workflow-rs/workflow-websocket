# WORKFLOW-WEBSOCKET

Part of the [WORKFLOW-RS](https://github.com/workflow-rs) application framework.

***

Platform-neutral WebSocket Client and Native Server

Platforms supported: Native (client & server), WASM (browser: client)

# Features:

* Uniform async Rust WebSocket client API that functions in the browser environment (backed by browser `WebSocket` class) as well as on native platforms (backed by [Tungstenite](https://crates.io/crates/async-tungstenite) client).
* Trait-based WebSocket server API backed by [Tungstenite](https://crates.io/crates/async-tungstenite) server.

This crate allows you to develop a WebSocket client that will work uniformly in in hte native environment and in-browser.

Workflow-WebSocket crate is currently (as of Q3 2022) one of the few available async Rust client-side in-browser WebSocket implementations.

This web socket crate offers an async message send API as well as provides access to Receive and Send async_std channels that can be used to send and receive WebSocket messages asynchronously.