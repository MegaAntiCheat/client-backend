//! An Event Loop abstraction that can be easily composed of message and handler
//! types, then run over some state.
//!
//! Messages can be any type (implementing [`Message<S>`] will allow the
//! message type to update the state once it has been appropriately handled),
//! and can be handled by any type implementing [`MessageHandler<S, IM, OM>`].
//! Message sources are of the form `Box<dyn MessageSource<M>>` and are generaly
//! something like a receiving channel for the message type, or one of the types
//! that make it up.
//!
//! The message and handler enum types can be constructed with
//! [`define_events!`] macro, where you spepcify the State type, the name of the
//! message and handler enum types, and all of the types they are composed of.
//!
//! # Examples
//!
//! ```
//! use std::sync::mpsc::{channel, Receiver, Sender};
//!
//! use event_loop::{define_events, try_get, EventLoop, MessageHandler, Is, Message};
//!
//! struct State {
//!     count: u32,
//! }
//!
//! struct Refresh;
//! impl Message<State> for Refresh {
//!     fn update_state(self, state: &mut State) { state.count = 0; }
//! }
//!
//! struct Increment;
//! impl Message<State> for Increment {
//!     fn update_state(self, state: &mut State) { state.count += 1; }
//! }
//!
//! struct Alert;
//! impl<IM, OM> MessageHandler<State, IM, OM> for Alert
//! where
//!     IM: Is<Increment> + Is<Refresh>,
//! {
//!     fn handle_message(&mut self, state: &State, message: &IM) -> Option<event_loop::Handled<OM>> {
//!         if try_get::<Increment>(message).is_some() {
//!             println!("Incrementing!");
//!         }
//!
//!         if try_get::<Refresh>(message).is_some() {
//!             println!("Refreshing. Final count: {}", state.count);
//!         }
//!
//!         None
//!     }
//! }
//!
//! define_events!(State, Message { Refresh, Increment }, Handler { Alert });
//!
//! #[tokio::main]
//! async fn main() {
//!     let (refresh_send, refresh_recv): (Sender<Refresh>, Receiver<Refresh>) = channel();
//!     let (sender, receiver): (Sender<Message>, Receiver<Message>) = channel();
//!
//!     let mut event_loop: EventLoop<State, Message, Handler> = EventLoop::new()
//!         .add_source(Box::new(refresh_recv))
//!         .add_source(Box::new(receiver))
//!         .add_handler(Alert);
//!
//!     let mut state = State { count: 0 };
//!
//!     for i in 0..10 {
//!         sender.send(Message::Increment(Increment)).ok();
//!
//!         if i % 3 == 0 {
//!             refresh_send.send(Refresh).ok();
//!         }
//!
//!         event_loop.execute_cycle(&mut state).await;
//!     }
//! }
//! ```
//!
//! **Output:**
//!
//! ```
//! Refreshing. Final count: 0
//! Incrementing!
//! Incrementing!
//! Incrementing!
//! Refreshing. Final count: 3
//! Incrementing!
//! Incrementing!
//! Incrementing!
//! Refreshing. Final count: 3
//! Incrementing!
//! Incrementing!
//! Incrementing!
//! Refreshing. Final count: 3
//! Incrementing!
//! ```

use std::{future::Future, marker::PhantomData};

use futures::future::BoxFuture;
use tokio::{sync::mpsc::UnboundedReceiver, task::JoinHandle};

pub struct EventLoop<S, M, H>
where
    S: Send,
    M: Send + Message<S> + 'static,
    H: MessageHandler<S, M, M>,
{
    pub sources: Vec<Box<dyn MessageSource<M> + 'static + Send>>,
    pub handlers: Vec<H>,
    pub queue: Vec<M>,
    pub async_tasks: Vec<JoinHandle<Option<M>>>,

    state: PhantomData<S>,
}

impl<S, M, H> EventLoop<S, M, H>
where
    S: Send,
    M: Send + Message<S> + 'static,
    H: MessageHandler<S, M, M>,
{
    #[must_use]
    pub fn new() -> Self {
        Self {
            sources: Vec::new(),
            handlers: Vec::new(),
            queue: Vec::new(),
            async_tasks: Vec::new(),
            state: PhantomData,
        }
    }

    #[must_use]
    pub fn add_source(mut self, source: Box<dyn MessageSource<M> + Send>) -> Self {
        self.sources.push(source);
        self
    }

    #[must_use]
    pub fn add_handler(mut self, handler: impl Into<H>) -> Self {
        self.handlers.push(handler.into());
        self
    }

    pub fn handle_message(&mut self, mut message: M, state: &mut S) -> Vec<Action<M>> {
        let mut out = Vec::new();

        message.preprocess(state);

        for h in &mut self.handlers {
            match h.handle_message(state, &message) {
                Some(Handled(Internal::Single(m))) => out.push(m),
                Some(Handled(Internal::Batch(ms))) => out.extend(ms),
                None => {}
            }
        }

        message.update_state(state);

        out
    }

    pub fn handle_messages(&mut self, messages: Vec<M>, state: &mut S) -> Vec<Action<M>> {
        let mut out = Vec::new();

        for m in messages {
            out.extend(self.handle_message(m, state));
        }
        out
    }

    #[allow(clippy::future_not_send)]
    pub async fn execute_cycle(&mut self, state: &mut S) -> Option<()> {
        let mut messages = Vec::new();

        // Check sources
        messages.append(&mut self.queue);
        for s in &mut self.sources {
            while let Some(m) = s.next_message() {
                messages.push(m);
            }
        }

        // Check async tasks
        let mut finished_tasks = Vec::new();
        for (i, j) in self.async_tasks.iter_mut().enumerate() {
            if j.is_finished() {
                finished_tasks.push(i);
                match j.await {
                    Ok(Some(m)) => messages.push(m),
                    Ok(None) => {}
                    Err(e) => {
                        tracing::error!("Task paniced: {e}");
                    }
                }
            }
        }
        for i in finished_tasks.into_iter().rev() {
            self.async_tasks.remove(i);
        }

        if messages.is_empty() {
            return None;
        }

        // Handle messages
        for a in self.handle_messages(messages, state) {
            match a {
                Action::Message(m) => self.queue.push(m),
                Action::Future(f) => {
                    self.async_tasks.push(tokio::task::spawn(f));
                }
            }
        }

        Some(())
    }
}

pub trait MessageHandler<S, IM, OM> {
    fn handle_message(&mut self, state: &S, message: &IM) -> Option<Handled<OM>>;
}

impl<S, IM, OM, T> MessageHandler<S, IM, OM> for &T {
    fn handle_message(&mut self, _state: &S, _message: &IM) -> Option<Handled<OM>> {
        None
    }
}

impl<S, M, H> Default for EventLoop<S, M, H>
where
    S: Send,
    M: Send + Message<S> + 'static,
    H: MessageHandler<S, M, M>,
{
    fn default() -> Self {
        Self::new()
    }
}

pub struct Handled<M>(Internal<Action<M>>);

enum Internal<T> {
    Single(T),
    Batch(Vec<T>),
}

pub enum Action<M> {
    Message(M),
    Future(BoxFuture<'static, Option<M>>),
}

impl<M> From<M> for Action<M> {
    fn from(value: M) -> Self {
        Self::Message(value)
    }
}

impl<M> Handled<M> {
    #[must_use]
    pub const fn none() -> Option<Self> {
        None
    }

    pub fn single(m: impl Into<M>) -> Option<Self> {
        Some(Self(Internal::Single(Action::Message(m.into()))))
    }

    pub fn future(future: impl Future<Output = Option<M>> + 'static + Send) -> Option<Self> {
        Some(Self(Internal::Single(Action::Future(Box::pin(future)))))
    }

    pub fn multiple(commands: impl IntoIterator<Item = Option<Self>>) -> Option<Self> {
        let mut batch = Vec::new();

        for maybe_handled in commands {
            match maybe_handled {
                None => {}
                Some(Self(Internal::Single(command))) => batch.push(command),
                Some(Self(Internal::Batch(commands))) => batch.extend(commands),
            }
        }

        Some(Self(Internal::Batch(batch)))
    }
}

#[allow(unused_variables)]
pub trait Message<S>: Sized {
    fn preprocess(&mut self, state: &S) {}
    fn update_state(self, state: &mut S) {}
}

// #[allow(unused_variables)]
// impl<M, S> Message<S> for &M {}

// #[allow(unused_variables)]
// impl<M, S> Message<S> for M {
//     fn preprocess(&mut self, state: &S) {}
//     fn update_state(self, state: &mut S) {}
// }

pub trait MessageSource<M> {
    fn next_message(&mut self) -> Option<M>;
}

impl<M, I: Into<M>> MessageSource<M> for UnboundedReceiver<I> {
    fn next_message(&mut self) -> Option<M> {
        self.try_recv().map_or_else(|_| None, |m| Some(m.into()))
    }
}

impl<M, I: Into<M>> MessageSource<M> for std::sync::mpsc::Receiver<I> {
    fn next_message(&mut self) -> Option<M> {
        self.try_recv().map_or_else(|_| None, |m| Some(m.into()))
    }
}

impl<M, I: Into<M>> MessageSource<M> for tokio::sync::mpsc::Receiver<I> {
    fn next_message(&mut self) -> Option<M> {
        self.try_recv().map_or_else(|_| None, |m| Some(m.into()))
    }
}

pub fn try_get<T>(message: &impl Is<T>) -> Option<&T> {
    message.try_get()
}

pub trait Is<T>: From<T> {
    fn is(&self) -> bool;
    fn try_get(&self) -> Option<&T>;
}

#[macro_export]
macro_rules! define_events {
    (
        $state:ty,
        $message_enum:ident { $($message:ident),+ $(,)? },
        $handler_enum:ident { $($handler:ident),+ $(,)? } $(,)?
    ) => {

        // ---------- Messages ---------

        // Define enum
        pub enum $message_enum {
            None,
            $($message($message)),+
        }

        // Impl update_state
        impl event_loop::Message<$state> for $message_enum {
            fn preprocess(&mut self, state: &$state) {
                use $message_enum::*;
                use event_loop::Message as MessageTrait;
                match self {
                    $message_enum::None => {},
                    $($message(i) => i.preprocess(state)),+
                }
            }
            fn update_state(self, state: &mut $state) {
                use $message_enum::*;
                use event_loop::Message as MessageTrait;
                match self {
                    $message_enum::None => {},
                    $($message(i) => i.update_state(state)),+
                }
            }
        }

        // Impl Is
        $(
            impl event_loop::Is<$message> for $message_enum {
                fn is(&self) -> bool {
                    match self {
                        $message_enum::$message(_) => true,
                        _ => false,
                    }
                }
                fn try_get(&self) -> Option<&$message> {
                    match self {
                        $message_enum::$message(a) => Some(a),
                        _ => None,
                    }
                }
            }
        )+

        // Impl Into
        $(
            impl From<$message> for $message_enum {
                fn from(val: $message) -> $message_enum {
                    $message_enum::$message(val)
                }
            }
        )+

        // Impl TryInto
        $(
            impl TryInto<$message> for $message_enum {
                type Error = ();
                fn try_into(self) -> Result<$message, Self::Error> {
                    match self {
                        $message_enum::$message(out) => Ok(out),
                        _ => Err(())
                    }
                }
            }
        )+

        // -------------- Handlers ------------

        // Handler enum
        pub enum $handler_enum {
            $($handler($handler)),+
        }

        // Impl MessageHandler<State, Message>
        impl event_loop::MessageHandler<$state, $message_enum, $message_enum> for $handler_enum {
            fn handle_message(&mut self, state: &$state, message: &$message_enum) -> Option<event_loop::Handled<$message_enum>> {
                match self {
                    $($handler_enum::$handler(inner) => inner.handle_message(state, message)),+
                }
            }
        }

        $(
            impl From<$handler> for $handler_enum {
                fn from(val: $handler) -> Self {
                    Self::$handler(val)
                }
            }
        )+
    };
}
