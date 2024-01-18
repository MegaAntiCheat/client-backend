use std::{future::Future, marker::PhantomData, sync::mpsc::Receiver};

use futures::future::BoxFuture;
use tokio::{sync::mpsc::UnboundedReceiver, task::JoinHandle};

pub struct EventLoop<S, M, H>
where
    S: Send,
    M: Send + ConsumingStateUpdater<S> + 'static,
    H: HandlerStruct<S, M>,
{
    pub sources: Vec<Box<dyn MessageSource<M>>>,
    pub handlers: Vec<H>,
    pub queue: Vec<M>,
    pub async_tasks: Vec<JoinHandle<M>>,

    state: PhantomData<S>,
}

impl<S, M, H> EventLoop<S, M, H>
where
    S: Send,
    M: Send + ConsumingStateUpdater<S> + 'static,
    H: HandlerStruct<S, M>,
{
    pub fn new() -> EventLoop<S, M, H> {
        EventLoop {
            sources: Vec::new(),
            handlers: Vec::new(),
            queue: Vec::new(),
            async_tasks: Vec::new(),
            state: PhantomData,
        }
    }

    pub fn add_source(mut self, source: Box<dyn MessageSource<M>>) -> Self {
        self.sources.push(source);
        self
    }

    pub fn add_handler(mut self, handler: impl Into<H>) -> Self {
        self.handlers.push(handler.into());
        self
    }

    pub async fn execute_cycle(&mut self, state: &mut S) {
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
                messages.push(j.await.unwrap());
            }
        }
        for i in finished_tasks.into_iter().rev() {
            self.async_tasks.remove(i);
        }

        // Run handlers
        for h in &mut self.handlers {
            for m in &messages {
                let mut actions = Vec::new();
                match h.handle_message(state, m).0 {
                    Internal::None => {}
                    Internal::Single(a) => actions.push(a),
                    Internal::Batch(a) => actions = a,
                }

                for a in actions {
                    match a {
                        Action::Message(m) => self.queue.push(m),
                        Action::Future(f) => {
                            self.async_tasks.push(tokio::task::spawn(f));
                        }
                    }
                }
            }
        }

        // Update state
        for m in messages {
            m.update_state(state);
        }
    }
}

pub trait HandlerStruct<S, M> {
    fn handle_message(&mut self, state: &S, message: &M) -> Handled<M>;
}

pub struct Handled<M>(Internal<Action<M>>);

enum Internal<T> {
    None,
    Single(T),
    Batch(Vec<T>),
}

pub enum Action<M> {
    Message(M),
    Future(BoxFuture<'static, M>),
}

impl<M> From<M> for Action<M> {
    fn from(value: M) -> Self {
        Action::Message(value)
    }
}

impl<M> Handled<M> {
    pub const fn none() -> Self {
        Self(Internal::None)
    }

    pub fn single(m: impl Into<M>) -> Self {
        Self(Internal::Single(Action::Message(m.into())))
    }

    pub fn future(future: impl Future<Output = M> + 'static + Send) -> Self {
        Self(Internal::Single(Action::Future(Box::pin(future))))
    }

    pub fn multiple(commands: impl IntoIterator<Item = Handled<M>>) -> Self {
        let mut batch = Vec::new();

        for Handled(command) in commands {
            match command {
                Internal::None => {}
                Internal::Single(command) => batch.push(command),
                Internal::Batch(commands) => batch.extend(commands),
            }
        }

        Self(Internal::Batch(batch))
    }
}

pub trait StateUpdater<S> {
    fn update_state(&self, state: &mut S) {}
}

pub trait ConsumingStateUpdater<S>
where
    Self: Sized,
{
    fn update_state(self, state: &mut S) {}
}

pub trait MessageSource<M> {
    fn next_message(&mut self) -> Option<M>;
}

impl<M, I: Into<M>> MessageSource<M> for UnboundedReceiver<I> {
    fn next_message(&mut self) -> Option<M> {
        match self.try_recv() {
            Ok(m) => Some(m.into()),
            _ => None,
        }
    }
}

impl<M, I: Into<M>> MessageSource<M> for Receiver<I> {
    fn next_message(&mut self) -> Option<M> {
        match self.try_recv() {
            Ok(m) => Some(m.into()),
            _ => None,
        }
    }
}

pub trait Is<T> {
    fn is(&self) -> bool;
}

// Define Handler struct
#[macro_export]
macro_rules! define_handlers {
    ($enum:ident <$state:ty, $message:ty>: $($handler:ident),+) => {
        // Define enum
        pub enum $enum {
            $($handler($handler)),+
        }

        // Impl HandlerStruct<State, Message>
        impl HandlerStruct<$state, $message> for $enum {
            fn handle_message(&mut self, state: &$state, message: &$message) -> Handled<$message> {
                match self {
                    $($enum::$handler(inner) => inner.handle_message(state, message)),+
                }
            }
        }

        $(
            impl From<$handler> for $enum {
                fn from(val: $handler) -> Self {
                    Self::$handler(val)
                }
            }
        )+
    };
}

// Define Message struct
#[macro_export]
macro_rules! define_messages {
    ($enum:ident <$state:ty>: $($message:ident),+) => {
        // Define enum
        pub enum $enum {
            $($message($message)),+
        }

        // Impl update_state
        impl events::ConsumingStateUpdater<$state> for $enum {
            fn update_state(self, state: &mut $state) {
                use $enum::*;
                match self {
                    $($message(i) => i.update_state(state)),+
                }
            }
        }

        // Impl Is
        $(
            impl events::Is<$message> for $enum {
                fn is(&self) -> bool {
                    match self {
                        $enum::$message(_) => true,
                        _ => false,
                    }
                }
            }
        )+

        // Impl Into
        $(
            impl From<$message> for $enum {
                fn from(val: $message) -> $enum {
                    $enum::$message(val)
                }
            }
        )+

        // Impl TryInto
        $(
            impl TryInto<$message> for $enum {
                type Error = ();
                fn try_into(self) -> Result<$message, Self::Error> {
                    match self {
                        $enum::$message(out) => Ok(out),
                        _ => Err(())
                    }
                }
            }
        )+
    };
}
