//! The browser's WebSocket as a [`LiveLink`]: callbacks fill a queue that
//! the app drains once a frame. One binary message is one bota frame, as on
//! every carrier. There is no dialing again here: a page that wants the
//! seat back builds a fresh link and says its `Hello` again.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::rc::Rc;

use bota_proto::{ClientMsg, ServerMsg, encode_frame_to_vec};
use js_sys::Uint8Array;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;
use web_sys::{BinaryType, CloseEvent, Event, MessageEvent, WebSocket};

use crate::ui::LiveLink;
use crate::ws_frame::decode;

/// Messages held for the app at most; past them the link closes, since a
/// browser socket cannot be told to stop reading.
const MAX_INBOX: usize = 1024;

#[derive(Default)]
struct Shared {
    inbox: VecDeque<ServerMsg>,
    /// Frames sent before the socket opened.
    unopened: Vec<Vec<u8>>,
    open: bool,
    closed: bool,
}

/// A live match over a browser WebSocket (`ws://` or `wss://`).
pub struct WebSocketLink {
    socket: WebSocket,
    shared: Rc<RefCell<Shared>>,
    // Held so the callbacks live as long as the socket.
    _on_open: Closure<dyn FnMut(Event)>,
    _on_message: Closure<dyn FnMut(MessageEvent)>,
    _on_close: Closure<dyn FnMut(CloseEvent)>,
    _on_error: Closure<dyn FnMut(Event)>,
}

impl WebSocketLink {
    /// Starts connecting to `url`; messages sent meanwhile wait for the
    /// socket to open.
    pub fn connect(url: &str) -> std::io::Result<Self> {
        let socket = WebSocket::new(url).map_err(js_error)?;
        socket.set_binary_type(BinaryType::Arraybuffer);
        let shared = Rc::new(RefCell::new(Shared::default()));

        let on_open = {
            let (shared, socket) = (Rc::clone(&shared), socket.clone());
            Closure::<dyn FnMut(Event)>::new(move |_| {
                let mut shared = shared.borrow_mut();
                shared.open = true;
                for frame in std::mem::take(&mut shared.unopened) {
                    if socket.send_with_u8_array(&frame).is_err() {
                        shared.closed = true;
                    }
                }
            })
        };
        let on_message = {
            let (shared, socket) = (Rc::clone(&shared), socket.clone());
            Closure::<dyn FnMut(MessageEvent)>::new(move |event: MessageEvent| {
                let mut shared = shared.borrow_mut();
                let message = event
                    .data()
                    .dyn_into::<js_sys::ArrayBuffer>()
                    .map_err(|_| std::io::Error::other("a text message"))
                    .and_then(|buffer| decode(&Uint8Array::new(&buffer).to_vec()));
                match message {
                    Ok(ServerMsg::Ping) => {
                        let pong = encode_frame_to_vec(&ClientMsg::Pong).unwrap_or_default();
                        if socket.send_with_u8_array(&pong).is_err() {
                            shared.closed = true;
                        }
                    }
                    Ok(message) if shared.inbox.len() < MAX_INBOX => {
                        shared.inbox.push_back(message)
                    }
                    _ => {
                        shared.closed = true;
                        let _ = socket.close();
                    }
                }
            })
        };
        let closer = |shared: &Rc<RefCell<Shared>>| {
            let shared = Rc::clone(shared);
            move || shared.borrow_mut().closed = true
        };
        let (close, fail) = (closer(&shared), closer(&shared));
        let on_close = Closure::<dyn FnMut(CloseEvent)>::new(move |_| close());
        let on_error = Closure::<dyn FnMut(Event)>::new(move |_| fail());

        socket.set_onopen(Some(on_open.as_ref().unchecked_ref()));
        socket.set_onmessage(Some(on_message.as_ref().unchecked_ref()));
        socket.set_onclose(Some(on_close.as_ref().unchecked_ref()));
        socket.set_onerror(Some(on_error.as_ref().unchecked_ref()));
        Ok(Self {
            socket,
            shared,
            _on_open: on_open,
            _on_message: on_message,
            _on_close: on_close,
            _on_error: on_error,
        })
    }
}

fn js_error(error: wasm_bindgen::JsValue) -> std::io::Error {
    std::io::Error::other(format!("WebSocket: {error:?}"))
}

impl LiveLink for WebSocketLink {
    fn poll(&mut self) -> Vec<ServerMsg> {
        self.shared.borrow_mut().inbox.drain(..).collect()
    }

    fn send(&mut self, msg: &ClientMsg) -> std::io::Result<()> {
        let frame = encode_frame_to_vec(msg).map_err(std::io::Error::other)?;
        let mut shared = self.shared.borrow_mut();
        if shared.closed {
            return Err(std::io::Error::new(
                std::io::ErrorKind::BrokenPipe,
                "the WebSocket is closed",
            ));
        }
        if !shared.open {
            shared.unopened.push(frame);
            return Ok(());
        }
        self.socket.send_with_u8_array(&frame).map_err(|error| {
            shared.closed = true;
            js_error(error)
        })
    }

    fn is_closed(&self) -> bool {
        self.shared.borrow().closed
    }
}

impl Drop for WebSocketLink {
    fn drop(&mut self) {
        self.socket.set_onopen(None);
        self.socket.set_onmessage(None);
        self.socket.set_onclose(None);
        self.socket.set_onerror(None);
        let _ = self.socket.close();
    }
}
